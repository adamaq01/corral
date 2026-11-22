use std::collections::{HashMap, VecDeque};
use std::hash::Hash;
use std::net::{IpAddr, Ipv6Addr};
use std::num::NonZeroUsize;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

/// The network a client address counts against.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Network(IpAddr);

// Dual-stack listeners report IPv4 peers as ::ffff:a.b.c.d, and an IPv6 client usually holds
// a whole /64.
impl From<IpAddr> for Network {
    fn from(address: IpAddr) -> Self {
        Self(match address.to_canonical() {
            IpAddr::V6(v6) => IpAddr::V6(Ipv6Addr::from_bits(v6.to_bits() & (u128::MAX << 64))),
            v4 @ IpAddr::V4(_) => v4,
        })
    }
}

/// Refuses a key once it has `limit` failures within `window`.
pub struct FailureWindow<K> {
    limit: NonZeroUsize,
    window: Duration,
    failures: Mutex<HashMap<K, VecDeque<Failure>>>,
}

#[derive(Clone, PartialEq, Eq)]
struct Failure {
    at: Instant,
    source: Option<String>,
}

#[must_use = "dropping an attempt keeps it counted as a failure"]
pub struct Attempt<'a, K: Eq + Hash> {
    window: &'a FailureWindow<K>,
    recorded: Vec<(K, Failure)>,
}

impl<K: Clone + Eq + Hash> FailureWindow<K> {
    #[must_use]
    pub fn new(limit: NonZeroUsize, window: Duration) -> Self {
        Self {
            limit,
            window,
            failures: Mutex::default(),
        }
    }

    /// Refuses when any key is full, else counts a failure against every key until the attempt
    /// is released, so concurrent guesses cannot all pass while the first are being checked.
    /// Attempts naming the same `source` count once per key.
    pub fn admit(&self, keys: &[K], source: Option<&str>, now: Instant) -> Option<Attempt<'_, K>> {
        let mut failures = self.failures();
        failures.retain(|_, log| {
            log.retain(|failure| now.saturating_duration_since(failure.at) < self.window);
            !log.is_empty()
        });
        if keys.iter().any(|key| {
            failures
                .get(key)
                .is_some_and(|log| log.len() >= self.limit.get())
        }) {
            return None;
        }
        let mut recorded = Vec::new();
        for key in keys {
            let log = failures.entry(key.clone()).or_default();
            if source.is_some()
                && log
                    .iter()
                    .any(|failure| failure.source.as_deref() == source)
            {
                continue;
            }
            let failure = Failure {
                at: now,
                source: source.map(str::to_owned),
            };
            log.push_back(failure.clone());
            recorded.push((key.clone(), failure));
        }
        Some(Attempt {
            window: self,
            recorded,
        })
    }
}

impl<K: Eq + Hash> FailureWindow<K> {
    fn failures(&self) -> MutexGuard<'_, HashMap<K, VecDeque<Failure>>> {
        self.failures.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl<K: Eq + Hash> Attempt<'_, K> {
    pub fn release(self) {
        let mut failures = self.window.failures();
        for (key, failure) in self.recorded {
            let Some(log) = failures.get_mut(&key) else {
                continue;
            };
            if let Some(index) = log.iter().position(|logged| *logged == failure) {
                log.remove(index);
            }
            if log.is_empty() {
                failures.remove(&key);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;
    use std::num::NonZeroUsize;
    use std::time::{Duration, Instant};

    use super::{FailureWindow, Network};

    const WINDOW: Duration = Duration::from_mins(15);

    fn window() -> FailureWindow<&'static str> {
        FailureWindow::new(NonZeroUsize::new(3).unwrap(), WINDOW)
    }

    fn fail(window: &FailureWindow<&'static str>, keys: &[&'static str], now: Instant) -> bool {
        window.admit(keys, None, now).is_some()
    }

    fn network(address: &str) -> Network {
        Network::from(address.parse::<IpAddr>().unwrap())
    }

    #[test]
    fn a_full_key_refuses_every_attempt_that_names_it() {
        let (window, now) = (window(), Instant::now());
        for i in 1..=3 {
            assert!(fail(&window, &["adam", "10.0.0.1"], now), "{i}");
        }
        assert!(!fail(&window, &["adam"], now));
        assert!(!fail(&window, &["eve", "10.0.0.1"], now));
        assert!(fail(&window, &["eve", "10.0.0.2"], now));
    }

    #[test]
    fn a_refused_attempt_counts_against_no_key() {
        let (window, now) = (window(), Instant::now());
        for _ in 1..=3 {
            assert!(fail(&window, &["adam"], now));
        }
        for _ in 1..=5 {
            assert!(!fail(&window, &["adam", "10.0.0.1"], now));
        }
        assert!(fail(&window, &["10.0.0.1"], now));
    }

    #[test]
    fn attempts_in_flight_count_until_released() {
        let (window, now) = (window(), Instant::now());
        let in_flight: Vec<_> = (0..3)
            .map(|_| window.admit(&["adam"], None, now).unwrap())
            .collect();
        assert!(!fail(&window, &["adam"], now));
        in_flight.into_iter().for_each(super::Attempt::release);
        for _ in 1..=20 {
            window.admit(&["adam"], None, now).unwrap().release();
        }
        assert!(fail(&window, &["adam"], now));
    }

    #[test]
    fn attempts_from_one_source_count_once() {
        let (window, now) = (window(), Instant::now());
        for _ in 1..=10 {
            assert!(window.admit(&["10.0.0.1"], Some("conn-1"), now).is_some());
        }
        window
            .admit(&["10.0.0.1"], Some("conn-1"), now)
            .unwrap()
            .release();
        assert!(window.admit(&["10.0.0.1"], Some("conn-2"), now).is_some());
        assert!(window.admit(&["10.0.0.1"], Some("conn-3"), now).is_some());
        assert!(window.admit(&["10.0.0.1"], Some("conn-1"), now).is_none());
        assert!(window.admit(&["10.0.0.1"], Some("conn-4"), now).is_none());
    }

    #[test]
    fn failures_expire_after_the_window() {
        let (window, start) = (window(), Instant::now());
        for i in 1..=3 {
            assert!(fail(&window, &["adam"], start + Duration::from_secs(i)));
        }
        assert!(!fail(&window, &["adam"], start + WINDOW));
        let first_expired = start + Duration::from_secs(1) + WINDOW;
        assert!(fail(&window, &["adam"], first_expired));
        assert!(!fail(&window, &["adam"], first_expired));
    }

    #[test]
    fn expired_keys_are_forgotten() {
        let window = FailureWindow::new(NonZeroUsize::MIN, WINDOW);
        let start = Instant::now();
        for i in 0..100 {
            assert!(window.admit(&[i], None, start).is_some());
        }
        assert!(window.admit(&[1000], None, start + WINDOW).is_some());
        assert_eq!(window.failures().len(), 1);
    }

    #[test]
    fn ipv6_counts_per_64_and_mapped_ipv4_per_address() {
        assert_eq!(network("2001:db8::1"), network("2001:db8::ffff:6"));
        assert_ne!(network("2001:db8::1"), network("2001:db8:0:1::1"));
        assert_eq!(network("::ffff:192.0.2.1"), network("192.0.2.1"));
        assert_ne!(network("::ffff:192.0.2.1"), network("192.0.2.2"));
    }
}
