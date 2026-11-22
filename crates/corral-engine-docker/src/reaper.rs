use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use bollard::models::ContainerSummary;
use chrono::{DateTime, Utc};
use corral_activity::{
    ActivityProbe, HerdrProbe, IdleDecision, SshSessionProbe, any_active, decide,
};
use corral_core::Result;
use corral_core::domain::SandboxId;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::DockerEngine;
use crate::container::{self, SANDBOX_ID};
use crate::docker_exec::DockerExec;

const SSH_SESSION: &str = "sshd: sandbox";

/// When each running sandbox was last seen active. Kept only in memory: a restart of Corral
/// costs at most one extra idle period.
#[derive(Default)]
pub(crate) struct LastActive(Mutex<HashMap<SandboxId, DateTime<Utc>>>);

impl LastActive {
    pub(crate) fn touch(&self, id: &SandboxId, at: DateTime<Utc>) {
        self.entries().insert(id.clone(), at);
    }

    fn since(&self, id: &SandboxId, now: DateTime<Utc>) -> DateTime<Utc> {
        *self.entries().entry(id.clone()).or_insert(now)
    }

    fn retain(&self, running: &[SandboxId]) {
        self.entries().retain(|id, _| running.contains(id));
    }

    fn entries(&self) -> MutexGuard<'_, HashMap<SandboxId, DateTime<Utc>>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl DockerEngine {
    #[must_use]
    pub fn spawn_lifecycle(
        &self,
        interval: Duration,
        shutdown: CancellationToken,
    ) -> JoinHandle<()> {
        let engine = self.clone();
        tokio::spawn(async move {
            let probes: [Box<dyn ActivityProbe>; 2] = [
                Box::new(SshSessionProbe::new(SSH_SESSION)),
                Box::new(HerdrProbe),
            ];
            loop {
                tokio::select! {
                    () = shutdown.cancelled() => return,
                    () = tokio::time::sleep(interval) => {}
                }
                tokio::select! {
                    () = shutdown.cancelled() => return,
                    result = engine.sweep(&probes) => {
                        if let Err(err) = result {
                            warn!(error = %err, "idle sweep failed");
                        }
                    }
                }
            }
        })
    }

    async fn sweep(&self, probes: &[Box<dyn ActivityProbe>]) -> Result<()> {
        let running: Vec<SandboxId> = self
            .running()
            .await?
            .iter()
            .filter_map(sandbox_id)
            .collect();
        self.last_active.retain(&running);
        for id in &running {
            if let Err(err) = self.reap(probes, id, self.config.idle_timeout).await {
                warn!(
                    sandbox = id.0.as_str(),
                    error = %err,
                    "could not put an idle sandbox to sleep"
                );
            }
        }
        Ok(())
    }

    async fn reap(
        &self,
        probes: &[Box<dyn ActivityProbe>],
        id: &SandboxId,
        timeout: Duration,
    ) -> Result<()> {
        let exec = DockerExec::new(self.docker.clone(), container::name(id));
        let activity = any_active(probes, &exec).await;
        match &activity {
            Ok(true) => self.last_active.touch(id, Utc::now()),
            Ok(false) => {}
            Err(err) => {
                warn!(
                    sandbox = id.0.as_str(),
                    error = %err,
                    "activity probe failed, keeping the sandbox awake"
                );
            }
        }
        let _lock = self.locks.lock(id).await;
        let now = Utc::now();
        if decide(activity, self.last_active.since(id, now), timeout, now) == IdleDecision::Sleep {
            self.stop_container(id).await?;
            info!(sandbox = id.0.as_str(), "put an idle sandbox to sleep");
        }
        Ok(())
    }
}

fn sandbox_id(container: &ContainerSummary) -> Option<SandboxId> {
    let id = container.labels.as_ref()?.get(SANDBOX_ID)?;
    Some(SandboxId(id.clone()))
}
