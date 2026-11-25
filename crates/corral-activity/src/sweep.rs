use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use corral_core::Result;
use corral_core::domain::SandboxId;
use tokio::sync::{Mutex, MutexGuard};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::{Exec, IdleDecision, decide, is_active};

/// What the idle sweep needs from an engine.
#[async_trait]
pub trait Sweepable: Send + Sync + 'static {
    /// What `suspend` needs from the listing, such as a version to suspend against.
    type Listed: Send + Sync;
    type Exec: Exec;

    fn activity(&self) -> &ActivityLock;
    async fn running(&self) -> Result<Vec<(SandboxId, Self::Listed)>>;
    fn exec(&self, id: &SandboxId) -> Self::Exec;
    /// Runs while wakes are held off.
    async fn suspend(&self, id: &SandboxId, listed: &Self::Listed) -> Result<()>;
}

/// When each running sandbox was last seen active, behind the engine-wide lock that wakes and
/// idle suspends take. Kept only in memory: a restart of Corral costs at most one extra idle
/// period.
#[derive(Default)]
pub struct ActivityLock(Mutex<HashMap<SandboxId, DateTime<Utc>>>);

/// Held by a wake from its capacity check to its start. Idle suspends and other wakes wait for
/// it, so no sandbox is put to sleep under a wake and concurrent wakes cannot pass the running
/// limit together.
pub struct Waking<'a> {
    _entries: MutexGuard<'a, HashMap<SandboxId, DateTime<Utc>>>,
}

impl ActivityLock {
    pub async fn wake(&self, id: &SandboxId) -> Waking<'_> {
        let mut entries = self.0.lock().await;
        entries.insert(id.clone(), Utc::now());
        Waking { _entries: entries }
    }
}

pub fn spawn<E: Sweepable>(
    engine: Arc<E>,
    interval: Duration,
    idle_timeout: Duration,
    shutdown: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let sweeps = async {
            loop {
                tokio::time::sleep(interval).await;
                sweep(engine.as_ref(), idle_timeout).await;
            }
        };
        shutdown.run_until_cancelled(sweeps).await;
    })
}

async fn sweep<E: Sweepable>(engine: &E, timeout: Duration) {
    let running = match engine.running().await {
        Ok(running) => running,
        Err(error) => {
            warn!(%error, "could not list running sandboxes for the idle sweep");
            return;
        }
    };
    engine
        .activity()
        .0
        .lock()
        .await
        .retain(|id, _| running.iter().any(|(listed, _)| listed == id));
    for (id, listed) in &running {
        let sandbox = id.0.as_str();
        let activity = is_active(&engine.exec(id)).await;
        if let Err(error) = &activity {
            warn!(sandbox, %error, "activity probe failed, keeping the sandbox awake");
        }
        let mut last_active = engine.activity().0.lock().await;
        let now = Utc::now();
        if matches!(activity, Ok(true)) {
            last_active.insert(id.clone(), now);
        }
        let since = *last_active.entry(id.clone()).or_insert(now);
        if decide(activity, since, timeout, now) == IdleDecision::Sleep {
            match engine.suspend(id, listed).await {
                Ok(()) => info!(sandbox, "put an idle sandbox to sleep"),
                Err(error) => warn!(sandbox, %error, "could not put an idle sandbox to sleep"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex as SyncMutex;

    use super::*;
    use crate::processes::tests::{LIST, listing};
    use crate::testing::ScriptedExec;

    const HERDR_LIST: &str = "/opt/corral/bin/hc ls --json";
    const CORE: (u32, u32, &str) = (35, 1, "/opt/corral/bin/herdr-core serve");
    const SESSION: (u32, u32, &str) = (42, 7, "sshd-session: sandbox@notty");

    struct Engine {
        activity: ActivityLock,
        sandboxes: Vec<(SandboxId, ScriptedExec)>,
        suspended: SyncMutex<Vec<SandboxId>>,
    }

    #[async_trait]
    impl Sweepable for Engine {
        type Listed = ();
        type Exec = ScriptedExec;

        fn activity(&self) -> &ActivityLock {
            &self.activity
        }

        async fn running(&self) -> Result<Vec<(SandboxId, ())>> {
            Ok(self
                .sandboxes
                .iter()
                .map(|(id, _)| (id.clone(), ()))
                .collect())
        }

        fn exec(&self, id: &SandboxId) -> ScriptedExec {
            let (_, exec) = self.sandboxes.iter().find(|(own, _)| own == id).unwrap();
            exec.clone()
        }

        async fn suspend(&self, id: &SandboxId, (): &()) -> Result<()> {
            self.suspended.lock().unwrap().push(id.clone());
            Ok(())
        }
    }

    fn engine(sandboxes: Vec<(&str, ScriptedExec)>) -> Engine {
        Engine {
            activity: ActivityLock::default(),
            sandboxes: sandboxes
                .into_iter()
                .map(|(id, exec)| (SandboxId(id.to_owned()), exec))
                .collect(),
            suspended: SyncMutex::default(),
        }
    }

    fn listed(processes: &[(u32, u32, &str)]) -> ScriptedExec {
        ScriptedExec::default().exits(LIST, 0, &listing(processes))
    }

    fn idle() -> ScriptedExec {
        listed(&[])
    }

    fn herdr(terminal_status: &str) -> ScriptedExec {
        listed(&[CORE]).exits(
            HERDR_LIST,
            0,
            &format!(r#"{{"terminals":[{{"state":"live","status":"{terminal_status}"}}]}}"#),
        )
    }

    async fn suspended_after_a_sweep(engine: &Engine, timeout: Duration) -> Vec<String> {
        sweep(engine, timeout).await;
        let suspended = engine.suspended.lock().unwrap();
        suspended.iter().map(|id| id.0.clone()).collect()
    }

    #[tokio::test]
    async fn an_idle_sandbox_sleeps_once_its_timeout_passes() {
        let expired = engine(vec![("idle", idle())]);
        let fresh = engine(vec![("idle", idle())]);

        assert_eq!(
            suspended_after_a_sweep(&expired, Duration::ZERO).await,
            ["idle"]
        );
        assert_eq!(
            suspended_after_a_sweep(&fresh, Duration::from_hours(1)).await,
            Vec::<String>::new()
        );
    }

    #[tokio::test]
    async fn only_idle_sandboxes_sleep() {
        let engine = engine(vec![
            ("session", listed(&[SESSION])),
            ("working", herdr("working")),
            ("idle", idle()),
            ("waiting", herdr("done")),
        ]);

        assert_eq!(
            suspended_after_a_sweep(&engine, Duration::ZERO).await,
            ["idle", "waiting"]
        );
    }

    #[tokio::test]
    async fn a_failed_probe_keeps_the_sandbox_awake() {
        let engine = engine(vec![
            ("no-exec", ScriptedExec::default().fails(LIST)),
            ("broken-herdr", listed(&[CORE]).fails(HERDR_LIST)),
        ]);

        assert_eq!(
            suspended_after_a_sweep(&engine, Duration::ZERO).await,
            Vec::<String>::new()
        );
    }
}
