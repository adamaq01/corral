use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use chrono::{DateTime, Utc};
use corral_core::domain::SandboxId;
use tokio::sync::{Mutex as AsyncMutex, MutexGuard as AsyncMutexGuard};
use tracing::info;

/// What is known of a running sandbox since it last booted. Kept only in memory: a restart of
/// Corral starts every sandbox's idle period again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Seen {
    /// Its last work from either source, or its boot.
    pub(crate) last_work: DateTime<Utc>,
    /// When it last reported what it used, if it has since Corral started.
    pub(crate) reported: Option<DateTime<Utc>>,
    /// Since when it is known, as booted or first seen.
    pub(crate) since: DateTime<Utc>,
    /// Whether the sweep said it keeps a sandbox that does not report awake.
    pub(crate) warned: bool,
}

impl Seen {
    pub(crate) fn since(now: DateTime<Utc>) -> Self {
        Self {
            last_work: now,
            reported: None,
            since: now,
            warned: false,
        }
    }
}

/// What is known of each running sandbox, and the engine-wide lock that wakes and idle suspends
/// take. Clones share both.
#[derive(Clone, Default)]
pub struct ActivityLock {
    wakes: Arc<AsyncMutex<()>>,
    seen: Arc<Mutex<HashMap<SandboxId, Seen>>>,
}

/// Held by a wake from its capacity check to its start. Idle suspends and other wakes wait for
/// it, so no sandbox is put to sleep under a wake and concurrent wakes cannot pass the running
/// limit together.
pub struct Waking<'a> {
    _held: AsyncMutexGuard<'a, ()>,
    lock: &'a ActivityLock,
    id: SandboxId,
}

impl Waking<'_> {
    /// The wake started the sandbox from asleep: its idle period starts now.
    pub fn booted(&self) {
        self.lock
            .seen()
            .insert(self.id.clone(), Seen::since(Utc::now()));
    }
}

impl ActivityLock {
    pub async fn wake(&self, id: &SandboxId) -> Waking<'_> {
        let held = if let Ok(held) = self.wakes.try_lock() {
            held
        } else {
            let began = Instant::now();
            let held = self.wakes.lock().await;
            let ms = began.elapsed().as_millis();
            info!(sandbox = %id.0, ms, "a wake waited on the running limit");
            held
        };
        Waking {
            _held: held,
            lock: self,
            id: id.clone(),
        }
    }

    /// Held while idle sandboxes are put to sleep.
    pub(crate) async fn suspending(&self) -> AsyncMutexGuard<'_, ()> {
        self.wakes.lock().await
    }

    pub(crate) fn seen(&self) -> MutexGuard<'_, HashMap<SandboxId, Seen>> {
        self.seen.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use corral_core::domain::SandboxId;
    use corral_core::fakes::Logs;

    use super::ActivityLock;

    #[tokio::test]
    async fn a_wake_that_waits_for_another_is_logged_with_its_wait() {
        let lock = ActivityLock::default();
        let logs = Logs::capture();
        let first = lock.wake(&SandboxId("adam-main-1a2b3c".to_owned())).await;
        assert!(logs.lines().is_empty(), "{:#?}", logs.lines());
        let second = async {
            drop(lock.wake(&SandboxId("eve-main-4d5e6f".to_owned())).await);
        };
        let release = async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            drop(first);
        };
        tokio::join!(second, release);

        let waited = logs.one("a wake waited on the running limit");
        assert!(waited.starts_with(" INFO"), "{waited}");
        let ms: u64 = waited.split(" ms=").nth(1).unwrap().parse().unwrap();
        assert!(waited.contains(" sandbox=eve-main-4d5e6f "), "{waited}");
        assert!(ms >= 50, "{waited}");
    }
}
