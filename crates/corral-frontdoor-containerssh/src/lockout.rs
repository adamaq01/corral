use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::num::NonZeroUsize;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use tokio::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LockoutPolicy {
    pub max_failed_connections: NonZeroUsize,
    pub window: Duration,
}

impl Default for LockoutPolicy {
    fn default() -> Self {
        const TEN: NonZeroUsize = NonZeroUsize::new(10).unwrap();
        Self {
            max_failed_connections: TEN,
            window: Duration::from_mins(5),
        }
    }
}

pub struct Lockout {
    policy: LockoutPolicy,
    failures: Mutex<HashMap<IpAddr, VecDeque<Failure>>>,
}

struct Failure {
    connection_id: String,
    at: Instant,
}

impl Lockout {
    pub fn new(policy: LockoutPolicy) -> Self {
        Self {
            policy,
            failures: Mutex::new(HashMap::new()),
        }
    }

    pub fn is_locked(&self, address: IpAddr) -> bool {
        let now = Instant::now();
        self.failures().get_mut(&address).is_some_and(|log| {
            self.forget_expired(log, now);
            log.len() >= self.policy.max_failed_connections.get()
        })
    }

    pub fn record_failure(&self, address: IpAddr, connection_id: &str) {
        let now = Instant::now();
        let mut failures = self.failures();
        failures.retain(|_, log| {
            self.forget_expired(log, now);
            !log.is_empty()
        });
        let log = failures.entry(address).or_default();
        if !log
            .iter()
            .any(|failure| failure.connection_id == connection_id)
        {
            log.push_back(Failure {
                connection_id: connection_id.to_owned(),
                at: now,
            });
        }
    }

    fn forget_expired(&self, log: &mut VecDeque<Failure>, now: Instant) {
        log.retain(|failure| now.duration_since(failure.at) < self.policy.window);
    }

    fn failures(&self) -> MutexGuard<'_, HashMap<IpAddr, VecDeque<Failure>>> {
        self.failures.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
