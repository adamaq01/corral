use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use corral_core::domain::{SandboxId, SandboxState, SandboxStatus};
use corral_core::ports::{Env, Readiness, SandboxEngine, SandboxSpec, SshTarget};
use corral_core::{Error, Result};
use k8s_openapi::api::core::v1::{PersistentVolumeClaim, Pod, Secret};
use kube::api::{DeleteParams, ListParams, Patch, PatchParams, PostParams};
use kube::{Api, Client};
use serde_json::json;
use tokio::time::{Instant, sleep};

use crate::AgentSandboxConfig;
use crate::crd::{OperatingMode, Sandbox};
use crate::error::kube_error;
use crate::manifest::{self, LAST_ACTIVE, MANAGED_BY, MANAGER, SANDBOX_ID};
use crate::status;

const POLL_INTERVAL: Duration = Duration::from_secs(1);

pub struct AgentSandboxEngine {
    sandboxes: Api<Sandbox>,
    secrets: Api<Secret>,
    claims: Api<PersistentVolumeClaim>,
    pods: Api<Pod>,
    config: AgentSandboxConfig,
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
        }
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
            .filter(|sandbox| sandbox.spec.operating_mode == OperatingMode::Running)
            .count())
    }

    async fn at_capacity(&self, already_running: bool) -> Result<bool> {
        let running = self.running_count().await?;
        Ok(at_capacity(
            running,
            self.config.max_running,
            already_running,
        ))
    }

    async fn merge_patch(&self, id: &SandboxId, patch: serde_json::Value) -> Result<()> {
        self.sandboxes
            .patch(&id.0, &PatchParams::default(), &Patch::Merge(patch))
            .await
            .map_err(kube_error)?;
        Ok(())
    }
}

fn at_capacity(running: usize, max: usize, already_running: bool) -> bool {
    !already_running && running >= max
}

#[async_trait]
impl SandboxEngine for AgentSandboxEngine {
    async fn create(&self, spec: SandboxSpec) -> Result<SandboxId> {
        let id = manifest::new_id();
        let mut sandbox = manifest::build(&id, &spec, &self.config, Utc::now());
        if self.at_capacity(false).await? {
            sandbox.spec.operating_mode = OperatingMode::Suspended;
        }
        self.write_env(&id, &spec.env).await?;
        if let Err(error) = self
            .sandboxes
            .create(&PostParams::default(), &sandbox)
            .await
        {
            let _ = self
                .secrets
                .delete_opt(&manifest::secret_name(&id), &DeleteParams::default())
                .await;
            return Err(kube_error(error));
        }
        Ok(id)
    }

    async fn status(&self, id: &SandboxId) -> Result<SandboxStatus> {
        self.observe(id).await
    }

    async fn ensure_running(
        &self,
        id: &SandboxId,
        env: Env,
        deadline: Duration,
    ) -> Result<Readiness> {
        let started = Instant::now();
        let sandbox = self.sandbox(id).await?;
        if self
            .at_capacity(sandbox.spec.operating_mode == OperatingMode::Running)
            .await?
        {
            return Err(Error::LimitReached(format!(
                "at most {} sandboxes may run at once",
                self.config.max_running
            )));
        }
        self.write_env(id, &env).await?;
        self.merge_patch(
            id,
            json!({
                "metadata": {"annotations": {LAST_ACTIVE: Utc::now().to_rfc3339()}},
                "spec": {"operatingMode": OperatingMode::Running},
            }),
        )
        .await?;
        loop {
            if self.observe(id).await?.state == SandboxState::Running {
                return Ok(Readiness::Ready);
            }
            let remaining = deadline.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return Ok(Readiness::StillWaking);
            }
            sleep(POLL_INTERVAL.min(remaining)).await;
        }
    }

    async fn stop(&self, id: &SandboxId) -> Result<()> {
        self.merge_patch(
            id,
            json!({"spec": {"operatingMode": OperatingMode::Suspended}}),
        )
        .await
    }

    async fn restart(&self, id: &SandboxId, env: Env) -> Result<()> {
        self.sandbox(id).await?;
        self.write_env(id, &env).await?;
        self.pods
            .delete_opt(&id.0, &DeleteParams::default())
            .await
            .map_err(kube_error)?;
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

    async fn ssh_target(&self, id: &SandboxId) -> Result<SshTarget> {
        match self.observe(id).await?.state {
            SandboxState::Running => Ok(SshTarget::KubernetesPod {
                namespace: self.config.namespace.clone(),
                pod: id.0.clone(),
            }),
            state => Err(Error::Engine(format!(
                "sandbox {} is {state:?}, not running",
                id.0
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::at_capacity;

    #[test]
    fn running_capacity_is_enforced() {
        assert!(!at_capacity(5, 6, false));
        assert!(at_capacity(6, 6, false));
        assert!(at_capacity(7, 6, false));
        assert!(!at_capacity(6, 6, true));
        assert!(at_capacity(0, 0, false));
    }
}
