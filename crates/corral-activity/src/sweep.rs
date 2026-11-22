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

use crate::{Exec, IdleDecision, Sessions, decide, is_active};

/// What the idle sweep needs from an engine.
#[async_trait]
pub trait Sweepable: Send + Sync + 'static {
    const SESSIONS: Sessions;
    /// What `suspend` needs from the listing, such as a version to suspend against.
    type Listed: Send + Sync;
    type Exec: Exec;

    fn activity(&self) -> &ActivityLock;
    fn idle_timeout(&self) -> Duration;
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
    shutdown: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let sweeps = async {
            loop {
                tokio::time::sleep(interval).await;
                sweep(engine.as_ref()).await;
            }
        };
        shutdown.run_until_cancelled(sweeps).await;
    })
}

async fn sweep<E: Sweepable>(engine: &E) {
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
    let timeout = engine.idle_timeout();
    for (id, listed) in &running {
        let sandbox = id.0.as_str();
        let activity = is_active(E::SESSIONS, &engine.exec(id)).await;
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
    use crate::testing::ScriptedExec;

    const SSHD: &str = "pgrep -f sshd: sandbox";
    const HERDR_CORE: &str = "pgrep -x herdr-core";
    const HERDR_LIST: &str = "hc ls --json";

    struct Engine {
        activity: ActivityLock,
        idle_timeout: Duration,
        sandboxes: Vec<(SandboxId, ScriptedExec)>,
        suspended: SyncMutex<Vec<SandboxId>>,
    }

    #[async_trait]
    impl Sweepable for Engine {
        const SESSIONS: Sessions = Sessions::Sshd;
        type Listed = ();
        type Exec = ScriptedExec;

        fn activity(&self) -> &ActivityLock {
            &self.activity
        }

        fn idle_timeout(&self) -> Duration {
            self.idle_timeout
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

    fn engine(idle_timeout: Duration, sandboxes: Vec<(&str, ScriptedExec)>) -> Engine {
        Engine {
            activity: ActivityLock::default(),
            idle_timeout,
            sandboxes: sandboxes
                .into_iter()
                .map(|(id, exec)| (SandboxId(id.to_owned()), exec))
                .collect(),
            suspended: SyncMutex::default(),
        }
    }

    fn idle() -> ScriptedExec {
        ScriptedExec::default()
            .exits(SSHD, 1, "")
            .exits(HERDR_CORE, 1, "")
    }

    fn herdr(terminal_status: &str) -> ScriptedExec {
        ScriptedExec::default()
            .exits(SSHD, 1, "")
            .exits(HERDR_CORE, 0, "35\n")
            .exits(
                HERDR_LIST,
                0,
                &format!(r#"{{"terminals":[{{"state":"live","status":"{terminal_status}"}}]}}"#),
            )
    }

    async fn suspended_after_a_sweep(engine: &Engine) -> Vec<String> {
        sweep(engine).await;
        let suspended = engine.suspended.lock().unwrap();
        suspended.iter().map(|id| id.0.clone()).collect()
    }

    #[tokio::test]
    async fn an_idle_sandbox_sleeps_once_its_timeout_passes() {
        let expired = engine(Duration::ZERO, vec![("idle", idle())]);
        let fresh = engine(Duration::from_hours(1), vec![("idle", idle())]);

        assert_eq!(suspended_after_a_sweep(&expired).await, ["idle"]);
        assert_eq!(suspended_after_a_sweep(&fresh).await, Vec::<String>::new());
    }

    #[tokio::test]
    async fn only_idle_sandboxes_sleep() {
        let engine = engine(
            Duration::ZERO,
            vec![
                ("session", ScriptedExec::default().exits(SSHD, 0, "42\n")),
                ("working", herdr("working")),
                ("idle", idle()),
                ("waiting", herdr("done")),
            ],
        );

        assert_eq!(suspended_after_a_sweep(&engine).await, ["idle", "waiting"]);
    }

    #[tokio::test]
    async fn a_failed_probe_keeps_the_sandbox_awake() {
        let engine = engine(
            Duration::ZERO,
            vec![
                ("no-exec", ScriptedExec::default().fails(SSHD)),
                (
                    "broken-herdr",
                    ScriptedExec::default()
                        .exits(SSHD, 1, "")
                        .exits(HERDR_CORE, 0, "35\n")
                        .fails(HERDR_LIST),
                ),
            ],
        );

        assert_eq!(suspended_after_a_sweep(&engine).await, Vec::<String>::new());
    }
}
