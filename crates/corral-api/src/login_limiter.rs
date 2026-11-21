use std::collections::{HashMap, VecDeque};
use std::hash::Hash;
use std::net::{IpAddr, Ipv6Addr};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use corral_core::domain::Handle;

const MAX_FAILURES: usize = 5;
const WINDOW: Duration = Duration::from_mins(15);

#[derive(Default)]
pub(crate) struct LoginLimiter {
    failures: Mutex<Failures>,
}

#[derive(Default)]
struct Failures {
    by_address: FailureLog<IpAddr>,
    by_username: FailureLog<Handle>,
}

struct FailureLog<K>(HashMap<K, VecDeque<Instant>>);

impl<K> Default for FailureLog<K> {
    fn default() -> Self {
        Self(HashMap::new())
    }
}

#[must_use = "dropping an attempt keeps it counted as a failure"]
pub(crate) struct Attempt<'a> {
    limiter: &'a LoginLimiter,
    address: IpAddr,
    username: Option<Handle>,
    at: Instant,
}

impl LoginLimiter {
    // The attempt counts as a failure until `release`, so concurrent guesses cannot all pass
    // while the first ones are still being verified.
    pub(crate) fn admit(
        &self,
        username: &str,
        address: IpAddr,
        now: Instant,
    ) -> Option<Attempt<'_>> {
        let address = network(address);
        let username = Handle::parse(username).ok();
        let mut failures = self.failures();
        failures.by_address.prune(now);
        failures.by_username.prune(now);
        if failures.by_address.is_full(&address)
            || username
                .as_ref()
                .is_some_and(|username| failures.by_username.is_full(username))
        {
            return None;
        }
        failures.by_address.push(address, now);
        if let Some(username) = &username {
            failures.by_username.push(username.clone(), now);
        }
        Some(Attempt {
            limiter: self,
            address,
            username,
            at: now,
        })
    }

    fn failures(&self) -> MutexGuard<'_, Failures> {
        self.failures.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Attempt<'_> {
    pub(crate) fn release(self) {
        let mut failures = self.limiter.failures();
        failures.by_address.remove(&self.address, self.at);
        if let Some(username) = &self.username {
            failures.by_username.remove(username, self.at);
        }
    }
}

impl<K: Eq + Hash> FailureLog<K> {
    fn prune(&mut self, now: Instant) {
        self.0.retain(|_, log| {
            log.retain(|&at| now.saturating_duration_since(at) < WINDOW);
            !log.is_empty()
        });
    }

    fn is_full(&self, key: &K) -> bool {
        self.0.get(key).is_some_and(|log| log.len() >= MAX_FAILURES)
    }

    fn push(&mut self, key: K, at: Instant) {
        self.0.entry(key).or_default().push_back(at);
    }

    fn remove(&mut self, key: &K, at: Instant) {
        if let Some(log) = self.0.get_mut(key) {
            if let Some(index) = log.iter().position(|&logged| logged == at) {
                log.remove(index);
            }
            if log.is_empty() {
                self.0.remove(key);
            }
        }
    }
}

// Dual-stack listeners report IPv4 peers as ::ffff:a.b.c.d, and an IPv6 client usually holds
// a whole /64.
fn network(address: IpAddr) -> IpAddr {
    match address.to_canonical() {
        IpAddr::V6(v6) => IpAddr::V6(Ipv6Addr::from_bits(v6.to_bits() & (u128::MAX << 64))),
        v4 @ IpAddr::V4(_) => v4,
    }
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;
    use std::time::{Duration, Instant};

    use super::{LoginLimiter, WINDOW};

    fn ip(value: &str) -> IpAddr {
        value.parse().unwrap()
    }

    fn fail(limiter: &LoginLimiter, username: &str, address: &str, now: Instant) -> bool {
        limiter.admit(username, ip(address), now).is_some()
    }

    #[test]
    fn five_failures_per_username_from_any_address() {
        let (limiter, now) = (LoginLimiter::default(), Instant::now());
        for i in 1..=5 {
            assert!(fail(&limiter, "adam", &format!("10.0.0.{i}"), now), "{i}");
        }
        assert!(!fail(&limiter, "adam", "10.0.0.6", now));
        assert!(fail(&limiter, "eve", "10.0.0.6", now));
    }

    #[test]
    fn five_failures_per_address_across_usernames() {
        let (limiter, now) = (LoginLimiter::default(), Instant::now());
        for i in 1..=5 {
            assert!(fail(&limiter, &format!("user-{i}"), "10.0.0.1", now), "{i}");
        }
        assert!(!fail(&limiter, "user-6", "10.0.0.1", now));
        assert!(!fail(&limiter, "Not A Handle", "10.0.0.1", now));
        assert!(fail(&limiter, "user-6", "10.0.0.2", now));
    }

    #[test]
    fn ipv6_counts_per_64_and_mapped_ipv4_per_address() {
        let (limiter, now) = (LoginLimiter::default(), Instant::now());
        for i in 1..=5 {
            assert!(fail(
                &limiter,
                &format!("v6-{i}"),
                &format!("2001:db8::{i}"),
                now
            ));
            assert!(fail(&limiter, &format!("v4-{i}"), "::ffff:192.0.2.1", now));
        }
        assert!(!fail(&limiter, "v6-6", "2001:db8::ffff:6", now));
        assert!(fail(&limiter, "v6-6", "2001:db8:0:1::6", now));
        assert!(!fail(&limiter, "v4-6", "192.0.2.1", now));
        assert!(fail(&limiter, "v4-6", "::ffff:192.0.2.2", now));
    }

    #[test]
    fn usernames_that_are_not_handles_count_only_per_address() {
        let (limiter, now) = (LoginLimiter::default(), Instant::now());
        let long = "x".repeat(10_000);
        for i in 1..=6 {
            assert!(fail(&limiter, &long, &format!("10.0.1.{i}"), now), "{i}");
        }
    }

    #[test]
    fn released_attempts_are_not_counted() {
        let (limiter, now) = (LoginLimiter::default(), Instant::now());
        for i in 1..=20 {
            let attempt = limiter.admit("adam", ip("10.0.0.1"), now);
            attempt
                .expect("released attempts never fill the window")
                .release();
            assert!(limiter.failures().by_address.0.is_empty(), "{i}");
        }
    }

    #[test]
    fn attempts_in_flight_count_until_released() {
        let (limiter, now) = (LoginLimiter::default(), Instant::now());
        let in_flight: Vec<_> = (0..5)
            .map(|_| limiter.admit("adam", ip("10.0.0.1"), now).unwrap())
            .collect();
        assert!(limiter.admit("adam", ip("10.0.0.2"), now).is_none());
        in_flight.into_iter().for_each(super::Attempt::release);
        assert!(limiter.admit("adam", ip("10.0.0.2"), now).is_some());
    }

    #[test]
    fn failures_expire_after_the_window() {
        let (limiter, start) = (LoginLimiter::default(), Instant::now());
        for i in 1..=5 {
            assert!(fail(
                &limiter,
                "adam",
                "10.0.0.1",
                start + Duration::from_secs(i)
            ));
        }
        assert!(!fail(&limiter, "adam", "10.0.0.1", start + WINDOW));
        let first_expired = start + Duration::from_secs(1) + WINDOW;
        assert!(fail(&limiter, "adam", "10.0.0.1", first_expired));
        assert!(!fail(&limiter, "adam", "10.0.0.1", first_expired));
    }

    #[test]
    fn expired_entries_are_pruned() {
        let (limiter, start) = (LoginLimiter::default(), Instant::now());
        for i in 0..100 {
            assert!(fail(
                &limiter,
                &format!("user-{i}"),
                &format!("10.0.{i}.1"),
                start
            ));
        }
        assert!(fail(&limiter, "adam", "10.1.0.1", start + WINDOW));
        let failures = limiter.failures();
        assert_eq!(failures.by_address.0.len(), 1);
        assert_eq!(failures.by_username.0.len(), 1);
    }
}
