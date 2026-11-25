//! What the gate tells the server about work: the sandboxes a person or a client worked in
//! through it, gathered and sent every few seconds. Output alone, keepalives, window changes and
//! what a client adapter reads as passive are not work.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use corral_core::domain::SandboxId;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::client::ServerClient;

const REPORT_EVERY: Duration = Duration::from_secs(5);

/// The sandboxes worked in since the last report, across the gate.
#[derive(Clone, Default)]
pub(crate) struct WorkLog(Arc<Mutex<HashSet<SandboxId>>>);

impl WorkLog {
    /// Reports what was worked in to `server` until `shutdown`.
    pub(crate) fn report_to(&self, server: ServerClient, shutdown: CancellationToken) {
        let log = self.clone();
        tokio::spawn(shutdown.run_until_cancelled_owned(async move {
            loop {
                tokio::time::sleep(REPORT_EVERY).await;
                let worked: Vec<String> = log.take().into_iter().map(|id| id.0).collect();
                if worked.is_empty() {
                    continue;
                }
                if let Err(error) = server.worked(worked).await {
                    warn!(%error, "could not tell Corral which sandboxes were worked in");
                }
            }
        }));
    }

    /// Work in the sandbox `id`.
    pub(crate) fn of(&self, id: &SandboxId) -> Work {
        Work {
            log: self.clone(),
            sandbox: id.clone(),
        }
    }

    fn take(&self) -> HashSet<SandboxId> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

/// Notes work in one sandbox.
#[derive(Clone)]
pub(crate) struct Work {
    log: WorkLog,
    sandbox: SandboxId,
}

impl Work {
    pub(crate) fn sandbox(&self) -> &SandboxId {
        &self.sandbox
    }

    pub(crate) fn note(&self) {
        let mut seen = self.log.0.lock().unwrap_or_else(PoisonError::into_inner);
        if !seen.contains(&self.sandbox) {
            seen.insert(self.sandbox.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_sandbox_worked_in_is_reported_once_until_worked_in_again() {
        let log = WorkLog::default();
        let main = log.of(&SandboxId("main".to_owned()));
        let other = log.of(&SandboxId("other".to_owned()));
        main.note();
        main.note();
        other.note();

        let mut taken: Vec<String> = log.take().into_iter().map(|id| id.0).collect();
        taken.sort();
        assert_eq!(taken, ["main", "other"]);
        assert!(log.take().is_empty());
        main.note();
        assert_eq!(log.take().len(), 1);
    }
}
