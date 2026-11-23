use std::collections::HashMap;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use bollard::models::ContainerInspectResponse;
use bollard::query_parameters::{
    CreateContainerOptionsBuilder, ListContainersOptionsBuilder, RemoveContainerOptionsBuilder,
    RemoveVolumeOptions, UploadToContainerOptionsBuilder,
};
use bollard::{Docker, body_full};
use corral_activity::{ActivityLock, Exec};
use corral_core::domain::{SandboxId, SandboxStatus};
use corral_core::ports::labels::{MANAGED_BY, MANAGER, SANDBOX_ID};
use corral_core::ports::{
    Boot, Inspection, Readiness, SandboxEngine, SandboxSpec, SshTarget, Started,
};
use corral_core::{Error, Result};
use ssh_key::PublicKey;
use tracing::warn;

use crate::container;
use crate::docker_error::is_not_found;
use crate::docker_exec::DockerExec;
use crate::host_key::HostKey;
use crate::{DockerConfig, image, inspection, session_files, status};

const SSH_PORT: u16 = 22;
const SSH_USER: &str = "sandbox";
const READY_POLL: Duration = Duration::from_millis(250);
// sshd shows up in the process list before it listens; a TCP connect proves it accepts logins.
const SSHD_ACCEPTS: [&str; 3] = ["bash", "-c", "exec 3<>/dev/tcp/127.0.0.1/22"];

pub struct DockerEngine {
    pub(crate) docker: Docker,
    config: DockerConfig,
    proxy_public_key: PublicKey,
    max_running: usize,
    pub(crate) activity: ActivityLock,
}

impl DockerEngine {
    pub async fn connect(
        config: DockerConfig,
        proxy_public_key: PublicKey,
        max_running: usize,
    ) -> Result<Self> {
        let docker = Docker::connect_with_defaults()
            .map_err(|err| Error::internal("connect to Docker", err))?;
        match docker.inspect_network(&config.network, None).await {
            Ok(_) => {}
            Err(err) if is_not_found(&err) => {
                return Err(Error::internal(
                    format!("Docker network {:?}", config.network),
                    format!(
                        "does not exist: create it with `docker network create {network}` and \
                         attach only the SSH front door to it, never Corral",
                        network = config.network
                    ),
                ));
            }
            Err(err) => return Err(Error::internal("reach the Docker daemon", err)),
        }
        Ok(Self {
            docker,
            config,
            proxy_public_key,
            max_running,
            activity: ActivityLock::default(),
        })
    }

    pub(crate) async fn running_ids(&self) -> Result<Vec<SandboxId>> {
        let filters = HashMap::from([
            ("label", vec![format!("{MANAGED_BY}={MANAGER}")]),
            ("network", vec![self.config.network.clone()]),
            ("status", vec!["running".to_owned()]),
        ]);
        let options = ListContainersOptionsBuilder::default()
            .filters(&filters)
            .build();
        let running = self
            .docker
            .list_containers(Some(options))
            .await
            .map_err(|err| Error::internal("list running sandboxes", err))?;
        Ok(running
            .iter()
            .filter_map(|container| container.labels.as_ref()?.get(SANDBOX_ID))
            .map(|id| SandboxId(id.clone()))
            .collect())
    }

    async fn inspect(&self, id: &SandboxId) -> Result<Option<ContainerInspectResponse>> {
        let name = container::name(id);
        match self.docker.inspect_container(&name, None).await {
            Ok(found) => Ok(Some(found)),
            Err(err) if is_not_found(&err) => Ok(None),
            Err(err) => Err(Error::internal(format!("inspect {name}"), err)),
        }
    }

    // Runs under a wake, which holds other wakes off until the container has started.
    async fn boot(&self, id: &SandboxId, boot: &Boot) -> Result<()> {
        let max = self.max_running;
        if self.running_ids().await?.len() >= max {
            return Err(Error::LimitReached(format!(
                "{max} sandboxes are already running"
            )));
        }
        let name = container::name(id);
        let files = session_files::archive(&self.proxy_public_key, boot);
        let at_root = UploadToContainerOptionsBuilder::default().path("/").build();
        self.docker
            .upload_to_container(&name, Some(at_root), body_full(files.into()))
            .await
            .map_err(|err| Error::internal(format!("write the session files of {name}"), err))?;
        self.docker
            .start_container(&name, None)
            .await
            .map_err(|err| Error::internal(format!("start {name}"), err))
    }

    async fn wait_for_sshd(&self, id: &SandboxId, patience: Duration) -> Result<bool> {
        let exec = DockerExec::new(self.docker.clone(), container::name(id));
        let accepting = async {
            loop {
                if exec.run(&SSHD_ACCEPTS).await?.code == 0 {
                    return Ok(());
                }
                tokio::time::sleep(READY_POLL).await;
            }
        };
        match tokio::time::timeout(patience, accepting).await {
            Ok(accepted) => accepted.map(|()| true),
            Err(_) => Ok(false),
        }
    }
}

fn missing(id: &SandboxId) -> Error {
    Error::NotFound(format!("sandbox {}", id.0))
}

#[async_trait]
impl SandboxEngine for DockerEngine {
    async fn create(&self, id: &SandboxId, spec: &SandboxSpec) -> Result<()> {
        if self.inspect(id).await?.is_some() {
            return Ok(());
        }
        let body = container::body(id, spec, &self.config.network, &HostKey::generate())?;
        image::ensure_present(&self.docker, &spec.image).await?;
        self.docker
            .create_volume(container::volume(id, spec))
            .await
            .map_err(|err| Error::internal("create home volume", err))?;
        let name = container::name(id);
        let options = CreateContainerOptionsBuilder::default().name(&name).build();
        self.docker
            .create_container(Some(options), body)
            .await
            .map(drop)
            .map_err(|err| Error::internal(format!("create {name}"), err))
    }

    async fn start(&self, id: &SandboxId, boot: Boot, deadline: Duration) -> Result<Started> {
        let called = Instant::now();
        let (target, booted) = {
            let _waking = self.activity.wake(id).await;
            let current = self.inspect(id).await?.ok_or_else(|| missing(id))?;
            let booted = !status::is_running(&current);
            if booted {
                self.boot(id, &boot).await?;
            }
            let target = SshTarget::SshServer {
                host_key_fingerprint: container::host_key_fingerprint(id, &current)?,
                host: container::name(id),
                port: SSH_PORT,
                username: SSH_USER.to_owned(),
            };
            (target, booted)
        };
        let patience = deadline.saturating_sub(called.elapsed());
        let readiness = if self.wait_for_sshd(id, patience).await? {
            Readiness::Ready(target)
        } else {
            Readiness::Waking
        };
        Ok(Started { readiness, booted })
    }

    async fn stop(&self, id: &SandboxId) -> Result<()> {
        let name = container::name(id);
        match self.docker.stop_container(&name, None).await {
            Err(err) if is_not_found(&err) => Err(missing(id)),
            stopped => stopped.map_err(|err| Error::internal(format!("stop {name}"), err)),
        }
    }

    async fn delete(&self, id: &SandboxId) -> Result<()> {
        let name = container::name(id);
        let options = RemoveContainerOptionsBuilder::default().force(true).build();
        match self.docker.remove_container(&name, Some(options)).await {
            Err(err) if !is_not_found(&err) => {
                return Err(Error::internal(format!("remove {name}"), err));
            }
            _ => {}
        }
        let volume = container::volume_name(id);
        match self
            .docker
            .remove_volume(&volume, None::<RemoveVolumeOptions>)
            .await
        {
            Err(err) if !is_not_found(&err) => {
                Err(Error::internal(format!("remove volume {volume}"), err))
            }
            _ => Ok(()),
        }
    }

    async fn status(&self, id: &SandboxId) -> Result<SandboxStatus> {
        let observed = self.inspect(id).await;
        observed
            .and_then(|current| status::map(id, current.as_ref()))
            .inspect_err(|err| {
                if !matches!(err, Error::NotFound(_)) {
                    warn!(sandbox = id.0.as_str(), error = %err, "could not read sandbox status");
                }
            })
    }

    async fn inspect(&self, id: &SandboxId) -> Result<Inspection> {
        inspection::observe(&self.docker, id)
            .await
            .map(inspection::describe)
    }
}
