use std::collections::HashMap;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use bollard::models::{ContainerCreateBody, ContainerInspectResponse, NetworkCreateRequest};
use bollard::query_parameters::{
    CreateContainerOptionsBuilder, ListContainersOptionsBuilder, RemoveContainerOptionsBuilder,
    RemoveVolumeOptions, RenameContainerOptionsBuilder, UploadToContainerOptionsBuilder,
};
use bollard::{Docker, body_full};
use corral_activity::ActivityLock;
use corral_core::domain::{SandboxId, SandboxStatus};
use corral_core::ports::labels::{MANAGED_BY, MANAGER, SANDBOX_ID};
use corral_core::ports::{
    Boot, Inspection, Readiness, RegistryAccess, SandboxEngine, SandboxSpec, Started,
};
use corral_core::{Error, Result};
use tracing::{info, warn};

use crate::container;
use crate::docker_error::is_not_found;
use crate::docker_exec::DockerExec;
use crate::{DockerConfig, image, inspection, session_files, status};

const READY_POLL: Duration = Duration::from_millis(250);
/// `corral-init` marks the sandbox ready once its accounts and home are written.
const MARKED_READY: [&str; 4] = ["/opt/corral/bin/busybox", "test", "-e", "/run/corral/ready"];
// Image mounts, which bring the overlay into every sandbox, came with Docker Engine 28.
const IMAGE_MOUNTS: (u32, u32) = (1, 48);

pub struct DockerEngine {
    pub(crate) docker: Docker,
    config: DockerConfig,
    /// The image ID of the overlay, pinned when the engine connected.
    overlay: String,
    /// The credentials pulls use, for the registries that need them.
    registries: Vec<RegistryAccess>,
    max_running: usize,
    pub(crate) activity: ActivityLock,
}

impl DockerEngine {
    pub async fn connect(
        config: DockerConfig,
        max_running: usize,
        registries: Vec<RegistryAccess>,
    ) -> Result<Self> {
        let docker = Docker::connect_with_defaults()
            .map_err(|err| Error::internal("connect to Docker", err))?;
        match docker.inspect_network(&config.network, None).await {
            Ok(_) => {}
            Err(err) if is_not_found(&err) => {
                let network = NetworkCreateRequest {
                    name: config.network.clone(),
                    labels: Some(HashMap::from([(MANAGED_BY.to_owned(), MANAGER.to_owned())])),
                    ..NetworkCreateRequest::default()
                };
                docker.create_network(network).await.map_err(|err| {
                    Error::internal(format!("create the Docker network {}", config.network), err)
                })?;
                info!(
                    network = config.network,
                    "created the network sandboxes join"
                );
            }
            Err(err) => return Err(Error::internal("reach the Docker daemon", err)),
        }
        let version = docker
            .version()
            .await
            .map_err(|err| Error::internal("read the Docker daemon's version", err))?;
        let api = version.api_version.unwrap_or_default();
        if api_version(&api).is_none_or(|api| api < IMAGE_MOUNTS) {
            return Err(Error::internal(
                "the Docker daemon",
                format!(
                    "speaks API {api}: Corral needs Docker Engine 28 or later (API 1.48), whose \
                     image mounts bring the Corral overlay into sandboxes"
                ),
            ));
        }
        let overlay = image::refresh(&docker, &config.overlay_image, &registries).await?;
        Ok(Self {
            docker,
            config,
            overlay,
            registries,
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
        self.inspect_named(&container::name(id)).await
    }

    async fn inspect_named(&self, name: &str) -> Result<Option<ContainerInspectResponse>> {
        match self.docker.inspect_container(name, None).await {
            Ok(found) => Ok(Some(found)),
            Err(err) if is_not_found(&err) => Ok(None),
            Err(err) => Err(Error::internal(format!("inspect {name}"), err)),
        }
    }

    /// Finishes a recreate that stopped after removing the old container: its replacement is
    /// complete, and only lacks the sandbox's name.
    async fn settled(&self, id: &SandboxId) -> Result<Option<ContainerInspectResponse>> {
        if let Some(found) = self.inspect(id).await? {
            return Ok(Some(found));
        }
        let replacement = container::replacement_name(id);
        if self.inspect_named(&replacement).await?.is_none() {
            return Ok(None);
        }
        self.rename(&replacement, &container::name(id)).await?;
        self.inspect(id).await
    }

    /// Recreates a stopped container that is not what `create` would make today from `spec`, or
    /// without one from what it was made from, keeping its home volume. The replacement is
    /// complete before the old container goes.
    async fn converge(
        &self,
        id: &SandboxId,
        spec: Option<&SandboxSpec>,
        current: ContainerInspectResponse,
    ) -> Result<ContainerInspectResponse> {
        let recorded = container::Definition::recorded(id, &current)?;
        let definition = match spec {
            Some(spec) => container::Definition::new(id, spec)?,
            None => recorded,
        };
        let image_id =
            image::ensure_present(&self.docker, &definition.image, &self.registries).await?;
        let body = container::body(id, &definition, &self.wiring());
        if container::is_made_from(&current, &body, &image_id) {
            return Ok(current);
        }
        let (name, replacement) = (container::name(id), container::replacement_name(id));
        self.remove(&replacement, true).await?;
        self.create_container(&replacement, body).await?;
        self.remove(&name, false).await?;
        self.rename(&replacement, &name).await?;
        self.inspect(id).await?.ok_or_else(|| missing(id))
    }

    fn wiring(&self) -> container::Wiring<'_> {
        container::Wiring {
            network: &self.config.network,
            overlay: &self.overlay,
        }
    }

    async fn create_container(&self, name: &str, body: ContainerCreateBody) -> Result<()> {
        let options = CreateContainerOptionsBuilder::default().name(name).build();
        self.docker
            .create_container(Some(options), body)
            .await
            .map(drop)
            .map_err(|err| Error::internal(format!("create {name}"), err))
    }

    /// Without `force`, refuses a running container.
    async fn remove(&self, name: &str, force: bool) -> Result<()> {
        let options = RemoveContainerOptionsBuilder::default()
            .force(force)
            .build();
        match self.docker.remove_container(name, Some(options)).await {
            Err(err) if !is_not_found(&err) => Err(Error::internal(format!("remove {name}"), err)),
            _ => Ok(()),
        }
    }

    async fn rename(&self, from: &str, to: &str) -> Result<()> {
        let options = RenameContainerOptionsBuilder::default().name(to).build();
        self.docker
            .rename_container(from, options)
            .await
            .map_err(|err| Error::internal(format!("rename {from} to {to}"), err))
    }

    // Runs under a wake, which holds other wakes off until the container has started.
    async fn boot(
        &self,
        id: &SandboxId,
        spec: Option<&SandboxSpec>,
        current: ContainerInspectResponse,
        boot: &Boot,
    ) -> Result<ContainerInspectResponse> {
        let max = self.max_running;
        if self.running_ids().await?.len() >= max {
            return Err(Error::LimitReached(format!(
                "{max} sandboxes are already running"
            )));
        }
        let current = self.converge(id, spec, current).await?;
        let name = container::name(id);
        let files = session_files::archive(boot);
        let at_root = UploadToContainerOptionsBuilder::default().path("/").build();
        self.docker
            .upload_to_container(&name, Some(at_root), body_full(files.into()))
            .await
            .map_err(|err| Error::internal(format!("write the session files of {name}"), err))?;
        self.docker
            .start_container(&name, None)
            .await
            .map_err(|err| Error::internal(format!("start {name}"), err))?;
        Ok(current)
    }

    async fn wait_until_ready(&self, id: &SandboxId, patience: Duration) -> Result<bool> {
        let exec = DockerExec::new(self.docker.clone(), container::name(id));
        let ready = async {
            loop {
                if exec.run(&MARKED_READY).await? == 0 {
                    return Ok(());
                }
                tokio::time::sleep(READY_POLL).await;
            }
        };
        match tokio::time::timeout(patience, ready).await {
            Ok(ready) => ready.map(|()| true),
            Err(_) => Ok(false),
        }
    }
}

fn api_version(version: &str) -> Option<(u32, u32)> {
    let (major, minor) = version.split_once('.')?;
    Some((major.parse().ok()?, minor.parse().ok()?))
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
        let definition = container::Definition::new(id, spec)?;
        image::ensure_present(&self.docker, &spec.image, &self.registries).await?;
        self.docker
            .create_volume(container::volume(id, spec))
            .await
            .map_err(|err| Error::internal("create home volume", err))?;
        let body = container::body(id, &definition, &self.wiring());
        self.create_container(&container::name(id), body).await
    }

    async fn start(
        &self,
        id: &SandboxId,
        spec: Option<&SandboxSpec>,
        boot: Boot,
        deadline: Duration,
    ) -> Result<Started> {
        let called = Instant::now();
        let (current, booted) = {
            let waking = self.activity.wake(id).await;
            let mut current = self.settled(id).await?.ok_or_else(|| missing(id))?;
            let booted = !status::is_running(&current);
            if booted {
                current = Box::pin(self.boot(id, spec, current, &boot)).await?;
                waking.booted();
            }
            (current, booted)
        };
        // A container running since before it marked its readiness takes no session until its
        // next start, which the session's refusal says.
        let patience = deadline.saturating_sub(called.elapsed());
        let readiness = if !container::marks_readiness(&current)
            || self.wait_until_ready(id, patience).await?
        {
            Readiness::Ready
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
        for name in [container::name(id), container::replacement_name(id)] {
            self.remove(&name, true).await?;
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
