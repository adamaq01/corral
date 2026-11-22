use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use corral_core::domain::{SandboxId, SandboxStatus};
use corral_core::ports::{Env, Readiness, SandboxEngine, SandboxSpec, SshTarget};
use corral_core::{Error, Result};
use k8s_openapi::api::core::v1::{PersistentVolumeClaim, Pod, Secret};
use kube::api::{DeleteParams, ListParams, Patch, PatchParams, PostParams};
use kube::{Api, Client};
use serde_json::json;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::{Instant, sleep};
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::AgentSandboxConfig;
use crate::crd::{OperatingMode, Sandbox};
use crate::error::kube_error;
use crate::lifecycle::Reaper;
use crate::manifest::{self, LAST_ACTIVE, MANAGED_BY, MANAGER, SANDBOX_ID};
use crate::status;

const POLL_INTERVAL: Duration = Duration::from_secs(1);
const STOP_PATIENCE: Duration = Duration::from_mins(2);

pub struct AgentSandboxEngine {
    sandboxes: Api<Sandbox>,
    secrets: Api<Secret>,
    claims: Api<PersistentVolumeClaim>,
    pods: Api<Pod>,
    config: AgentSandboxConfig,
    // Held from the running count to the switch to Running, so concurrent starts cannot pass
    // max_running together.
    starts: Mutex<()>,
}

impl AgentSandboxEngine {
    #[must_use]
    pub fn new(client: Client, config: AgentSandboxConfig) -> Self {
        let namespace = config.namespace.as_str();
        Self {
            sandboxes: Api::namespaced(client.clone(), namespace),
            secrets: Api::namespaced(client.clone(), namespace),
            claims: Api::namespaced(client.clone(), namespace),
            pods: Api::namespaced(client, namespace),
            config,
            starts: Mutex::new(()),
        }
    }

    #[must_use]
    pub fn spawn_lifecycle(
        &self,
        interval: Duration,
        shutdown: CancellationToken,
    ) -> JoinHandle<()> {
        let reaper = Reaper::new(
            self.sandboxes.clone(),
            self.pods.clone(),
            self.config.idle_timeout,
        );
        tokio::spawn(reaper.run(interval, shutdown))
    }

    async fn sandbox(&self, id: &SandboxId) -> Result<Sandbox> {
        self.sandboxes.get(&id.0).await.map_err(kube_error)
    }

    async fn observe(&self, id: &SandboxId) -> Result<SandboxStatus> {
        let sandbox = self.sandbox(id).await?;
        let pod = self.pods.get_opt(&id.0).await.map_err(kube_error)?;
        Ok(status::map(&sandbox, pod.as_ref()))
    }

    async fn write_env(&self, id: &SandboxId, env: &Env) -> Result<()> {
        let secret = manifest::env_secret(id, env, &self.config.namespace);
        self.secrets
            .patch(
                &manifest::secret_name(id),
                &PatchParams::apply(MANAGER).force(),
                &Patch::Apply(&secret),
            )
            .await
            .map_err(kube_error)?;
        Ok(())
    }

    async fn running_count(&self) -> Result<usize> {
        let managed = ListParams::default().labels(&format!("{MANAGED_BY}={MANAGER}"));
        let sandboxes = self.sandboxes.list(&managed).await.map_err(kube_error)?;
        Ok(sandboxes
            .items
            .iter()
            .filter(|sandbox| {
                sandbox.spec.operating_mode == OperatingMode::Running
                    && sandbox.metadata.deletion_timestamp.is_none()
            })
            .count())
    }

    async fn resume(&self, id: &SandboxId, env: &Env) -> Result<()> {
        let _start = self.starts.lock().await;
        let sandbox = self.sandbox(id).await?;
        let mut patch =
            json!({"metadata": {"annotations": {LAST_ACTIVE: Utc::now().to_rfc3339()}}});
        if sandbox.spec.operating_mode != OperatingMode::Running {
            let max = self.config.max_running;
            if self.running_count().await? >= max {
                return Err(Error::LimitReached(format!(
                    "at most {max} sandboxes may run at once"
                )));
            }
            self.write_env(id, env).await?;
            patch["spec"] = json!({"operatingMode": OperatingMode::Running});
        }
        self.merge_patch(id, patch).await
    }

    async fn merge_patch(&self, id: &SandboxId, patch: serde_json::Value) -> Result<()> {
        self.sandboxes
            .patch(&id.0, &PatchParams::default(), &Patch::Merge(patch))
            .await
            .map_err(kube_error)?;
        Ok(())
    }
}

#[async_trait]
impl SandboxEngine for AgentSandboxEngine {
    async fn create(&self, id: &SandboxId, spec: &SandboxSpec) -> Result<()> {
        let sandbox = manifest::build(id, spec, &self.config, Utc::now());
        match self
            .sandboxes
            .create(&PostParams::default(), &sandbox)
            .await
        {
            Err(kube::Error::Api(status)) if status.is_already_exists() => Ok(()),
            created => created.map(drop).map_err(kube_error),
        }
    }

    async fn start(&self, id: &SandboxId, env: Env, deadline: Duration) -> Result<Readiness> {
        let started = Instant::now();
        self.resume(id, &env).await?;
        loop {
            if self.observe(id).await? == SandboxStatus::Running {
                return Ok(Readiness::Ready(SshTarget::KubernetesPod {
                    namespace: self.config.namespace.clone(),
                    pod: id.0.clone(),
                }));
            }
            let remaining = deadline.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return Ok(Readiness::Waking);
            }
            sleep(POLL_INTERVAL.min(remaining)).await;
        }
    }

    async fn stop(&self, id: &SandboxId) -> Result<()> {
        self.merge_patch(
            id,
            json!({"spec": {"operatingMode": OperatingMode::Suspended}}),
        )
        .await?;
        let started = Instant::now();
        while self
            .pods
            .get_opt(&id.0)
            .await
            .map_err(kube_error)?
            .is_some()
        {
            if started.elapsed() > STOP_PATIENCE {
                return Err(Error::internal(
                    format!("stop sandbox {}", id.0),
                    format!("its pod is still running after {STOP_PATIENCE:?}"),
                ));
            }
            sleep(POLL_INTERVAL).await;
        }
        Ok(())
    }

    async fn delete(&self, id: &SandboxId) -> Result<()> {
        let params = DeleteParams::default();
        self.sandboxes
            .delete_opt(&id.0, &params)
            .await
            .map_err(kube_error)?;
        self.secrets
            .delete_opt(&manifest::secret_name(id), &params)
            .await
            .map_err(kube_error)?;
        let claims = ListParams::default().labels(&format!("{SANDBOX_ID}={}", id.0));
        self.claims
            .delete_collection(&params, &claims)
            .await
            .map_err(kube_error)?;
        Ok(())
    }

    async fn status(&self, id: &SandboxId) -> Result<SandboxStatus> {
        self.observe(id).await.inspect_err(|err| {
            if !matches!(err, Error::NotFound(_)) {
                warn!(sandbox = id.0.as_str(), error = %err, "could not read sandbox status");
            }
        })
    }
}
