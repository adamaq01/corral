use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use bollard::Docker;
use bollard::models::{ContainerCreateBody, ContainerInspectResponse, ContainerSummary};
use bollard::query_parameters::{
    CreateContainerOptionsBuilder, ListContainersOptionsBuilder, RemoveContainerOptionsBuilder,
    RemoveVolumeOptions, RenameContainerOptionsBuilder,
};
use chrono::Utc;
use corral_activity::Exec;
use corral_core::domain::{SandboxId, SandboxStatus};
use corral_core::ports::{Env, Readiness, SandboxEngine, SandboxSpec, SshTarget};
use corral_core::{Error, Result};
use ssh_key::PublicKey;
use tokio::sync::Mutex;
use tracing::warn;

use crate::container::{self, Blueprint, MANAGED_BY, MANAGER};
use crate::docker_error::is_not_found;
use crate::docker_exec::DockerExec;
use crate::host_key::HostKey;
use crate::locks::{SandboxLock, SandboxLocks};
use crate::reaper::LastActive;
use crate::{DockerConfig, image, status};

const SSH_PORT: u16 = 22;
const SSH_USER: &str = "sandbox";
const READY_POLL: Duration = Duration::from_millis(250);
// sshd shows up in the process list before it listens; a TCP connect proves it accepts logins.
const SSHD_ACCEPTS: [&str; 3] = ["bash", "-c", "exec 3<>/dev/tcp/127.0.0.1/22"];

#[derive(Clone)]
pub struct DockerEngine {
    pub(crate) docker: Docker,
    pub(crate) config: Arc<DockerConfig>,
    pub(crate) last_active: Arc<LastActive>,
    pub(crate) locks: Arc<SandboxLocks>,
    // Held from the running count to the container start, so concurrent starts cannot pass
    // max_running together.
    starts: Arc<Mutex<()>>,
}

impl DockerEngine {
    pub async fn connect(config: DockerConfig) -> Result<Self> {
        PublicKey::from_openssh(&config.proxy_public_key).map_err(|err| {
            Error::Invalid(format!(
                "the front door's public key is not an OpenSSH key: {err}"
            ))
        })?;
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
            config: Arc::new(config),
            last_active: Arc::default(),
            locks: Arc::default(),
            starts: Arc::default(),
        })
    }

    pub(crate) async fn running(&self) -> Result<Vec<ContainerSummary>> {
        let filters = HashMap::from([
            ("label", vec![format!("{MANAGED_BY}={MANAGER}")]),
            ("network", vec![self.config.network.clone()]),
            ("status", vec!["running".to_owned()]),
        ]);
        let options = ListContainersOptionsBuilder::default()
            .filters(&filters)
            .build();
        self.docker
            .list_containers(Some(options))
            .await
            .map_err(|err| Error::internal("list running sandboxes", err))
    }

    pub(crate) async fn stop_container(&self, id: &SandboxId) -> Result<()> {
        let name = container::name(id);
        self.docker
            .stop_container(&name, None)
            .await
            .map_err(|err| Error::internal(format!("stop {name}"), err))
    }

    async fn observe(&self, id: &SandboxId) -> Result<SandboxStatus> {
        let current = match self.inspect(&container::name(id)).await? {
            Some(found) => Some(found),
            None => self.inspect(&container::staged_name(id)).await?,
        };
        status::map(id, current.as_ref())
    }

    async fn inspect(&self, name: &str) -> Result<Option<ContainerInspectResponse>> {
        match self.docker.inspect_container(name, None).await {
            Ok(found) => Ok(Some(found)),
            Err(err) if is_not_found(&err) => Ok(None),
            Err(err) => Err(Error::internal(format!("inspect {name}"), err)),
        }
    }

    // Finishes a replace that was interrupted after it removed the old container.
    async fn existing(&self, lock: &SandboxLock) -> Result<Option<ContainerInspectResponse>> {
        let id = lock.id();
        let name = container::name(id);
        if let Some(found) = self.inspect(&name).await? {
            return Ok(Some(found));
        }
        let staged = container::staged_name(id);
        if self.inspect(&staged).await?.is_some() {
            self.rename(&staged, &name).await?;
            return self.inspect(&name).await;
        }
        Ok(None)
    }

    async fn require(&self, lock: &SandboxLock) -> Result<ContainerInspectResponse> {
        self.existing(lock)
            .await?
            .ok_or_else(|| Error::NotFound(format!("sandbox {}", lock.id().0)))
    }

    async fn create_container(&self, name: &str, body: ContainerCreateBody) -> Result<()> {
        if let Some(reference) = &body.image {
            image::ensure_present(&self.docker, reference).await?;
        }
        let options = CreateContainerOptionsBuilder::default().name(name).build();
        self.docker
            .create_container(Some(options), body)
            .await
            .map(drop)
            .map_err(|err| Error::internal(format!("create {name}"), err))
    }

    // The new container is created before the old one is removed, so the sandbox's blueprint and
    // host key live in some container whatever point this is interrupted at.
    async fn replace(&self, lock: &SandboxLock, body: ContainerCreateBody) -> Result<()> {
        let name = container::name(lock.id());
        let staged = container::staged_name(lock.id());
        self.remove_container(&staged).await?;
        self.create_container(&staged, body).await?;
        self.remove_container(&name).await?;
        self.rename(&staged, &name).await
    }

    async fn rename(&self, from: &str, to: &str) -> Result<()> {
        let options = RenameContainerOptionsBuilder::default().name(to).build();
        self.docker
            .rename_container(from, options)
            .await
            .map_err(|err| Error::internal(format!("rename {from} to {to}"), err))
    }

    async fn remove_container(&self, name: &str) -> Result<()> {
        let options = RemoveContainerOptionsBuilder::default().force(true).build();
        match self.docker.remove_container(name, Some(options)).await {
            Err(err) if !is_not_found(&err) => Err(Error::internal(format!("remove {name}"), err)),
            _ => Ok(()),
        }
    }

    async fn boot(
        &self,
        lock: &SandboxLock,
        current: &ContainerInspectResponse,
        env: &Env,
    ) -> Result<()> {
        let id = lock.id();
        let _start = self.starts.lock().await;
        let max = self.config.max_running;
        if self.running().await?.len() >= max {
            return Err(Error::LimitReached(format!(
                "{max} sandboxes are already running"
            )));
        }
        let desired = Blueprint::read(id, current)?.build(env, &self.config);
        if container::env_hash(current) != Some(desired.env_hash.as_str()) {
            self.replace(lock, desired.body).await?;
        }
        let name = container::name(id);
        self.docker
            .start_container(&name, None)
            .await
            .map_err(|err| Error::internal(format!("start {name}"), err))
    }

    async fn wait_for_sshd(&self, id: &SandboxId, patience: Duration) -> Result<bool> {
        let exec = DockerExec::new(self.docker.clone(), container::name(id));
        let accepting = async {
            loop {
                let probe = exec.run(&SSHD_ACCEPTS).await.map_err(|err| {
                    Error::internal(format!("check sshd in sandbox {}", id.0), err)
                })?;
                if probe.code == 0 {
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

#[async_trait]
impl SandboxEngine for DockerEngine {
    async fn create(&self, id: &SandboxId, spec: &SandboxSpec) -> Result<()> {
        let lock = self.locks.lock(id).await;
        if self.existing(&lock).await?.is_some() {
            return Ok(());
        }
        let blueprint = Blueprint::new(id.clone(), spec, HostKey::generate())?;
        self.docker
            .create_volume(blueprint.volume())
            .await
            .map_err(|err| Error::internal("create home volume", err))?;
        self.create_container(
            &container::name(id),
            blueprint.build(&Env::new(), &self.config).body,
        )
        .await
    }

    async fn start(&self, id: &SandboxId, env: Env, deadline: Duration) -> Result<Readiness> {
        let called = Instant::now();
        let target = {
            let lock = self.locks.lock(id).await;
            let current = self.require(&lock).await?;
            if !status::is_running(&current) {
                self.boot(&lock, &current, &env).await?;
            }
            self.last_active.touch(id, Utc::now());
            SshTarget::SshServer {
                host_key_fingerprint: Blueprint::read(id, &current)?.fingerprint(),
                host: container::name(id),
                port: SSH_PORT,
                username: SSH_USER.to_owned(),
            }
        };
        let patience = deadline.saturating_sub(called.elapsed());
        if self.wait_for_sshd(id, patience).await? {
            Ok(Readiness::Ready(target))
        } else {
            Ok(Readiness::Waking)
        }
    }

    async fn stop(&self, id: &SandboxId) -> Result<()> {
        let lock = self.locks.lock(id).await;
        if status::is_running(&self.require(&lock).await?) {
            self.stop_container(id).await?;
        }
        Ok(())
    }

    async fn delete(&self, id: &SandboxId) -> Result<()> {
        let _lock = self.locks.lock(id).await;
        self.remove_container(&container::name(id)).await?;
        self.remove_container(&container::staged_name(id)).await?;
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
        self.observe(id).await.inspect_err(|err| {
            if !matches!(err, Error::NotFound(_)) {
                warn!(sandbox = id.0.as_str(), error = %err, "could not read sandbox status");
            }
        })
    }
}
