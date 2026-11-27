use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use corral_activity::ActivityLock;
use corral_core::domain::{SandboxId, SandboxStatus};
use corral_core::ports::labels::{MANAGED_BY, MANAGER, SANDBOX_ID};
use corral_core::ports::{
    Boot, Inspection, Readiness, RunningLimit, SandboxEngine, SandboxSpec, Started,
};
use corral_core::{Error, Result};
use k8s_openapi::api::core::v1::{Event, PersistentVolumeClaim, Pod, Secret};
use kube::api::{DeleteParams, ListParams, Patch, PatchParams, PostParams};
use kube::{Api, Client};
use serde_json::json;
use tokio::time::{Instant, sleep};
use tracing::{debug, warn};

use crate::AgentSandboxConfig;
use crate::crd::{OperatingMode, Sandbox};
use crate::error::kube_error;
use crate::inspection::{self, Observed};
use crate::manifest::{self, Workload};
use crate::status;

const POLL_INTERVAL: Duration = Duration::from_secs(1);
const STOP_PATIENCE: Duration = Duration::from_mins(2);

pub struct AgentSandboxEngine {
    pub(crate) sandboxes: Api<Sandbox>,
    secrets: Api<Secret>,
    claims: Api<PersistentVolumeClaim>,
    pub(crate) pods: Api<Pod>,
    events: Api<Event>,
    pub(crate) config: AgentSandboxConfig,
    running: Arc<dyn RunningLimit>,
    pub(crate) activity: ActivityLock,
}

impl AgentSandboxEngine {
    #[must_use]
    pub fn new(client: Client, config: AgentSandboxConfig, running: Arc<dyn RunningLimit>) -> Self {
        let namespace = config.namespace.as_str();
        Self {
            sandboxes: Api::namespaced(client.clone(), namespace),
            secrets: Api::namespaced(client.clone(), namespace),
            claims: Api::namespaced(client.clone(), namespace),
            pods: Api::namespaced(client.clone(), namespace),
            events: Api::namespaced(client, namespace),
            config,
            running,
            activity: ActivityLock::default(),
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

    async fn pod_uid(&self, id: &SandboxId) -> Result<Option<String>> {
        let pod = self.pods.get_opt(&id.0).await.map_err(kube_error)?;
        Ok(pod.and_then(|pod| pod.metadata.uid))
    }

    async fn write_boot(&self, id: &SandboxId, boot: &Boot, workload: &Workload) -> Result<()> {
        let namespace = &self.config.namespace;
        let home = manifest::home_secret(id, &boot.files, namespace, workload)?;
        let env = manifest::env_secret(id, &boot.env, namespace, workload);
        for secret in [env, home] {
            let name = secret.metadata.name.clone().unwrap_or_default();
            self.secrets
                .patch(
                    &name,
                    &PatchParams::apply(MANAGER).force(),
                    &Patch::Apply(&secret),
                )
                .await
                .map_err(kube_error)?;
            debug!(sandbox = %id.0, kind = %"Secret", %name, "applied a resource");
        }
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

    /// Whether it resumed the sandbox, which was suspended, with the pod template Corral would
    /// make for it today from `spec`, or without one from what it was made from.
    async fn resume(
        &self,
        id: &SandboxId,
        spec: Option<&SandboxSpec>,
        boot: &Boot,
    ) -> Result<bool> {
        let waking = self.activity.wake(id).await;
        let sandbox = self.sandbox(id).await?;
        if sandbox.spec.operating_mode == OperatingMode::Running {
            return Ok(false);
        }
        let max = self.running.max_running().await?;
        if self.running_count().await? >= max {
            return Err(Error::LimitReached(format!(
                "at most {max} sandboxes may run at once"
            )));
        }
        let workload = match spec {
            Some(spec) => Workload::new(id, spec),
            None => Workload::recorded(&sandbox)?,
        };
        self.write_boot(id, boot, &workload).await?;
        let template = manifest::pod_template(id, &workload, &self.config);
        // A replace drops what the template no longer has, which a merge patch would keep; one
        // patch for both means a resume never starts a pod from the old template.
        let resume: json_patch::Patch = serde_json::from_value(json!([
            {"op": "replace", "path": "/spec/podTemplate", "value": template},
            {"op": "replace", "path": "/spec/operatingMode", "value": OperatingMode::Running},
        ]))
        .expect("a well-formed JSON patch");
        self.sandboxes
            .patch(&id.0, &PatchParams::default(), &Patch::Json::<()>(resume))
            .await
            .map_err(kube_error)?;
        debug!(sandbox = %id.0, kind = %"Sandbox", name = %id.0, "patched a resource");
        waking.booted();
        Ok(true)
    }

    pub(crate) async fn merge_patch(&self, id: &SandboxId, patch: serde_json::Value) -> Result<()> {
        self.sandboxes
            .patch(&id.0, &PatchParams::default(), &Patch::Merge(patch))
            .await
            .map_err(kube_error)?;
        debug!(sandbox = %id.0, kind = %"Sandbox", name = %id.0, "patched a resource");
        Ok(())
    }
}

/// Logs `operation` on the sandbox: done at debug, failed at warn with the reason. A refusal,
/// such as the running limit, is its caller's to log.
fn logged<T>(operation: &str, id: &SandboxId, result: Result<T>) -> Result<T> {
    match &result {
        Ok(_) => debug!(%operation, sandbox = %id.0, "an agent-sandbox operation succeeded"),
        Err(error) if !error.is_refusal() => {
            let reason = error.to_string();
            warn!(%operation, sandbox = %id.0, reason, "an agent-sandbox operation failed");
        }
        Err(_) => {}
    }
    result
}

#[async_trait]
impl SandboxEngine for AgentSandboxEngine {
    async fn create(&self, id: &SandboxId, spec: &SandboxSpec) -> Result<()> {
        let sandbox = manifest::build(id, spec, &self.config);
        let created = match self
            .sandboxes
            .create(&PostParams::default(), &sandbox)
            .await
        {
            Err(kube::Error::Api(status)) if status.is_already_exists() => Ok(()),
            created => created.map(drop).map_err(kube_error),
        };
        logged("create", id, created)
    }

    async fn start(
        &self,
        id: &SandboxId,
        spec: Option<&SandboxSpec>,
        boot: Boot,
        deadline: Duration,
    ) -> Result<Started> {
        let woken = async {
            let started = Instant::now();
            let booted = self.resume(id, spec, &boot).await?;
            let readiness = loop {
                if self.observe(id).await? == SandboxStatus::Running {
                    break Readiness::Ready;
                }
                let remaining = deadline.saturating_sub(started.elapsed());
                if remaining.is_zero() {
                    break Readiness::Waking;
                }
                sleep(POLL_INTERVAL.min(remaining)).await;
            };
            Ok(Started { readiness, booted })
        };
        logged("start", id, woken.await)
    }

    async fn stop(&self, id: &SandboxId) -> Result<()> {
        // A wake during the stop, such as a client reconnecting, brings up a new pod under the
        // same name, so the stop waits only for the pod that ran before it.
        let stopped = async {
            let running = self.pod_uid(id).await?;
            self.merge_patch(
                id,
                json!({"spec": {"operatingMode": OperatingMode::Suspended}}),
            )
            .await?;
            let started = Instant::now();
            while running.is_some() && self.pod_uid(id).await? == running {
                if started.elapsed() > STOP_PATIENCE {
                    return Err(Error::internal(
                        format!("stop sandbox {}", id.0),
                        format!("its pod is still running after {STOP_PATIENCE:?}"),
                    ));
                }
                sleep(POLL_INTERVAL).await;
            }
            Ok(())
        };
        logged("stop", id, stopped.await)
    }

    async fn delete(&self, id: &SandboxId) -> Result<()> {
        let deleted = async {
            let params = DeleteParams::default();
            self.sandboxes
                .delete_opt(&id.0, &params)
                .await
                .map_err(kube_error)?;
            for secret in [manifest::secret_name(id), manifest::home_secret_name(id)] {
                self.secrets
                    .delete_opt(&secret, &params)
                    .await
                    .map_err(kube_error)?;
            }
            let claims = ListParams::default().labels(&format!("{SANDBOX_ID}={}", id.0));
            self.claims
                .delete_collection(&params, &claims)
                .await
                .map_err(kube_error)?;
            Ok(())
        };
        logged("delete", id, deleted.await)
    }

    async fn status(&self, id: &SandboxId) -> Result<SandboxStatus> {
        self.observe(id).await.inspect_err(|err| {
            if !matches!(err, Error::NotFound(_)) {
                warn!(sandbox = id.0.as_str(), error = %err, "could not read sandbox status");
            }
        })
    }

    async fn inspect(&self, id: &SandboxId) -> Result<Inspection> {
        let claim = manifest::home_claim_name(id);
        // The pod is named after its Sandbox, so one selector finds the events of both.
        let mut events = Vec::new();
        for name in [&id.0, &claim] {
            let about = ListParams::default().fields(&format!("involvedObject.name={name}"));
            events.extend(self.events.list(&about).await.map_err(kube_error)?.items);
        }
        Ok(inspection::describe(Observed {
            namespace: self.config.namespace.clone(),
            sandbox: self.sandboxes.get_opt(&id.0).await.map_err(kube_error)?,
            pod: self.pods.get_opt(&id.0).await.map_err(kube_error)?,
            claim: self.claims.get_opt(&claim).await.map_err(kube_error)?,
            events,
        }))
    }
}
