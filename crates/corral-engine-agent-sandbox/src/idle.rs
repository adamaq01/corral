use std::time::Duration;

use async_trait::async_trait;
use corral_activity::{LastActive, Sessions, Sweepable};
use corral_core::Result;
use corral_core::domain::{SandboxId, SandboxStatus};
use kube::ResourceExt;
use kube::api::ListParams;
use serde_json::json;

use crate::AgentSandboxEngine;
use crate::crd::OperatingMode;
use crate::error::kube_error;
use crate::manifest::{MANAGED_BY, MANAGER};
use crate::pod_exec::PodExec;
use crate::status;

#[async_trait]
impl Sweepable for AgentSandboxEngine {
    const SESSIONS: Sessions = Sessions::Exec;
    /// The resource version the sandbox was listed at.
    type Listed = Option<String>;
    type Exec = PodExec;

    fn last_active(&self) -> &LastActive {
        &self.last_active
    }

    fn idle_timeout(&self) -> Duration {
        self.config.idle_timeout
    }

    async fn running(&self) -> Result<Vec<(SandboxId, Option<String>)>> {
        let managed = ListParams::default().labels(&format!("{MANAGED_BY}={MANAGER}"));
        let sandboxes = self.sandboxes.list(&managed).await.map_err(kube_error)?;
        let mut running = Vec::new();
        for sandbox in sandboxes.items {
            let pod = self
                .pods
                .get_opt(&sandbox.name_any())
                .await
                .map_err(kube_error)?;
            if status::map(&sandbox, pod.as_ref()) == SandboxStatus::Running {
                running.push((SandboxId(sandbox.name_any()), sandbox.resource_version()));
            }
        }
        Ok(running)
    }

    fn exec(&self, id: &SandboxId) -> PodExec {
        PodExec {
            pods: self.pods.clone(),
            pod: id.0.clone(),
        }
    }

    // The listed resource version makes the suspend conditional, so any change since wins.
    async fn suspend(&self, id: &SandboxId, version: &Option<String>) -> Result<()> {
        self.merge_patch(
            id,
            json!({
                "metadata": {"resourceVersion": version},
                "spec": {"operatingMode": OperatingMode::Suspended},
            }),
        )
        .await
    }
}
