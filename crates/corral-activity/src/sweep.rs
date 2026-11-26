use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use corral_core::Result;
use corral_core::domain::{IdleLimits, Sandbox, SandboxId};
use corral_core::ports::SandboxRepo;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::ActivityLock;
use crate::decide::{Verdict, decide};
use crate::lock::Seen;

/// What the idle sweep needs from an engine.
#[async_trait]
pub trait Sweepable: Send + Sync + 'static {
    /// What `suspend` needs from the listing, such as a version to suspend against.
    type Listed: Send + Sync;

    fn activity(&self) -> &ActivityLock;
    async fn running(&self) -> Result<Vec<(SandboxId, Self::Listed)>>;
    /// Runs while wakes are held off.
    async fn suspend(&self, id: &SandboxId, listed: &Self::Listed) -> Result<()>;
}

#[derive(Clone, Copy, Debug)]
pub struct Settings {
    pub limits: IdleLimits,
    /// How often running sandboxes are checked.
    pub check_every: Duration,
    /// How often each sandbox reports what it used.
    pub report_every: Duration,
}

pub fn spawn<E: Sweepable>(
    engine: Arc<E>,
    sandboxes: Arc<dyn SandboxRepo>,
    settings: Settings,
    shutdown: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let sweeps = async {
            loop {
                tokio::time::sleep(settings.check_every).await;
                sweep(engine.as_ref(), sandboxes.as_ref(), &settings).await;
            }
        };
        shutdown.run_until_cancelled(sweeps).await;
    })
}

async fn sweep<E: Sweepable>(engine: &E, sandboxes: &dyn SandboxRepo, settings: &Settings) {
    let running = match engine.running().await {
        Ok(running) => running,
        Err(error) => {
            warn!(%error, "could not list running sandboxes for the idle sweep");
            return;
        }
    };
    let known: HashMap<SandboxId, Sandbox> = match sandboxes.list_all().await {
        Ok(known) => known
            .into_iter()
            .map(|sandbox| (sandbox.id.clone(), sandbox))
            .collect(),
        Err(error) => {
            warn!(%error, "could not read the sandboxes' idle settings");
            return;
        }
    };
    let lock = engine.activity();
    let _suspending = lock.suspending().await;
    let now = Utc::now();
    let sleepy: Vec<&(SandboxId, E::Listed)> = {
        let mut seen = lock.seen();
        seen.retain(|id, _| running.iter().any(|(listed, _)| listed == id));
        running
            .iter()
            .filter(|(id, _)| {
                let sandbox = known.get(id);
                let timeout = sandbox.map_or(settings.limits.default_timeout, |sandbox| {
                    settings.limits.timeout_of(sandbox)
                });
                let keep_awake_until = sandbox.and_then(|sandbox| sandbox.keep_awake_until);
                let entry = seen.entry(id.clone()).or_insert_with(|| Seen::since(now));
                let verdict = decide(entry, timeout, keep_awake_until, settings.report_every, now);
                let silent_for = now - entry.since;
                let long_silent = silent_for
                    .to_std()
                    .is_ok_and(|silent| silent > settings.report_every * 3);
                if verdict == Verdict::Unreported && long_silent && !entry.warned {
                    entry.warned = true;
                    warn!(
                        sandbox = id.0,
                        "the sandbox reports no activity, so it stays awake until it stops; \
                         one started before Corral was upgraded reports once restarted"
                    );
                }
                verdict == Verdict::Sleep
            })
            .collect()
    };
    for (id, listed) in sleepy {
        let sandbox = id.0.as_str();
        match engine.suspend(id, listed).await {
            Ok(()) => info!(sandbox, "put an idle sandbox to sleep"),
            Err(error) => warn!(sandbox, %error, "could not put an idle sandbox to sleep"),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex as SyncMutex;

    use chrono::TimeDelta;
    use corral_core::fakes::{MemorySandboxRepo, sandbox_name};
    use corral_core::ports::{ActivitySignals, Usage};

    use super::*;
    use crate::{Busy, Signals};

    const BUSY: Busy = Busy {
        cpu_percent: 5.0,
        network_bytes_per_sec: 4096,
    };
    const SETTINGS: Settings = Settings {
        limits: IdleLimits {
            default_timeout: Duration::from_mins(30),
            max_timeout: Duration::from_hours(4),
            max_keep_awake: Duration::from_hours(24),
        },
        check_every: Duration::from_mins(1),
        report_every: Duration::from_mins(1),
    };

    struct Engine {
        activity: ActivityLock,
        running: Vec<SandboxId>,
        suspended: SyncMutex<Vec<SandboxId>>,
    }

    #[async_trait]
    impl Sweepable for Engine {
        type Listed = ();

        fn activity(&self) -> &ActivityLock {
            &self.activity
        }

        async fn running(&self) -> Result<Vec<(SandboxId, ())>> {
            Ok(self.running.iter().map(|id| (id.clone(), ())).collect())
        }

        async fn suspend(&self, id: &SandboxId, (): &()) -> Result<()> {
            self.suspended.lock().unwrap().push(id.clone());
            Ok(())
        }
    }

    struct World {
        engine: Engine,
        repo: MemorySandboxRepo,
        signals: Signals,
    }

    impl World {
        async fn new(names: &[&str]) -> Self {
            let repo = MemorySandboxRepo::default();
            let mut running = Vec::new();
            for name in names {
                let sandbox = Sandbox {
                    id: SandboxId(format!("adam-{name}-000000")),
                    owner: corral_core::domain::UserId(uuid::Uuid::nil()),
                    name: sandbox_name(name),
                    image: corral_core::domain::SandboxImage::parse("default").unwrap(),
                    size: corral_core::domain::SizeKey::parse("small").unwrap(),
                    created_at: Utc::now(),
                    started_at: None,
                    setup_changed_at: None,
                    delete_at: None,
                    idle_timeout: None,
                    keep_awake_until: None,
                    repository: None,
                };
                repo.insert(&sandbox, 10).await.unwrap();
                running.push(sandbox.id);
            }
            let activity = ActivityLock::default();
            Self {
                signals: Signals::new(activity.clone(), BUSY, SETTINGS.report_every),
                engine: Engine {
                    activity,
                    running,
                    suspended: SyncMutex::default(),
                },
                repo,
            }
        }

        fn id(name: &str) -> SandboxId {
            SandboxId(format!("adam-{name}-000000"))
        }

        /// Moves every sandbox's last work and report back by `minutes`.
        fn age(&self, minutes: i64) {
            for seen in self.engine.activity.seen().values_mut() {
                seen.last_work -= TimeDelta::minutes(minutes);
                if let Some(reported) = &mut seen.reported {
                    *reported -= TimeDelta::minutes(minutes);
                }
            }
        }

        fn report(&self, name: &str, cpu_millis: u64, network_bytes: u64) {
            let usage = Usage {
                interval: Duration::from_mins(1),
                cpu: Duration::from_millis(cpu_millis),
                network_bytes,
            };
            self.signals.measured(&Self::id(name), usage);
        }

        async fn suspended_after_a_sweep(&self) -> Vec<String> {
            sweep(&self.engine, &self.repo, &SETTINGS).await;
            let suspended = self.engine.suspended.lock().unwrap();
            suspended
                .iter()
                .map(|id| id.0.split('-').nth(1).unwrap().to_owned())
                .collect()
        }
    }

    #[tokio::test]
    async fn only_sandboxes_idle_past_their_timeout_by_both_sources_sleep() {
        let w = World::new(&["typing", "busy", "streaming", "idle", "old"]).await;
        for name in ["typing", "busy", "streaming", "idle"] {
            w.report(name, 0, 0);
        }
        assert_eq!(w.suspended_after_a_sweep().await, Vec::<String>::new());

        w.age(31);
        w.report("typing", 0, 0);
        w.signals.worked(&[World::id("typing")]);
        w.report("busy", 30_000, 0);
        w.report("streaming", 0, 600_000);
        w.report("idle", 100, 1_000);
        assert_eq!(
            w.suspended_after_a_sweep().await,
            ["idle"],
            "old, which never reported, stays awake"
        );
    }

    #[tokio::test]
    async fn the_owners_timeout_and_keep_awake_apply() {
        let w = World::new(&["short", "long", "kept"]).await;
        w.repo
            .set_idle_timeout(&World::id("short"), Some(Duration::from_mins(5)))
            .await
            .unwrap();
        w.repo
            .set_idle_timeout(&World::id("long"), Some(Duration::from_hours(2)))
            .await
            .unwrap();
        w.repo
            .set_keep_awake_until(&World::id("kept"), Some(Utc::now() + TimeDelta::hours(1)))
            .await
            .unwrap();
        for name in ["short", "long", "kept"] {
            w.report(name, 0, 0);
        }
        w.suspended_after_a_sweep().await;
        w.age(40);
        for name in ["short", "long", "kept"] {
            w.report(name, 0, 0);
        }

        assert_eq!(w.suspended_after_a_sweep().await, ["short"]);
    }

    #[tokio::test]
    async fn a_boot_starts_the_idle_period_again() {
        let w = World::new(&["main"]).await;
        w.report("main", 0, 0);
        w.suspended_after_a_sweep().await;
        w.age(31);
        w.report("main", 0, 0);

        w.engine.activity.wake(&World::id("main")).await.booted();
        w.report("main", 0, 0);
        assert_eq!(w.suspended_after_a_sweep().await, Vec::<String>::new());
    }

    #[tokio::test]
    async fn a_sandbox_that_stops_reporting_stays_awake() {
        let w = World::new(&["main"]).await;
        w.report("main", 0, 0);
        w.suspended_after_a_sweep().await;
        w.age(31);

        assert_eq!(w.suspended_after_a_sweep().await, Vec::<String>::new());
    }
}
