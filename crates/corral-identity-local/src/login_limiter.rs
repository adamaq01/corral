use std::net::IpAddr;
use std::num::NonZeroUsize;
use std::time::{Duration, Instant};

use corral_core::app::{Attempt, FailureWindow, Network};

const MAX_FAILURES: NonZeroUsize = NonZeroUsize::new(5).unwrap();
const WINDOW: Duration = Duration::from_mins(15);
// Longer usernames count only against the address, so guesses cannot grow memory per byte.
const MAX_USERNAME_KEY: usize = 64;

#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) enum Key {
    Network(Network),
    Account(String),
}

/// Five failed logins per network and per username in fifteen minutes.
pub(crate) struct LoginLimiter(FailureWindow<Key>);

impl Default for LoginLimiter {
    fn default() -> Self {
        Self(FailureWindow::new(MAX_FAILURES, WINDOW))
    }
}

impl LoginLimiter {
    /// `None` once either limit is reached. The attempt counts as a failure unless released.
    pub(crate) fn admit(
        &self,
        username: &str,
        address: IpAddr,
        now: Instant,
    ) -> Option<Attempt<'_, Key>> {
        let mut keys = vec![Key::Network(Network::from(address))];
        if username.len() <= MAX_USERNAME_KEY {
            keys.push(Key::Account(username.to_owned()));
        }
        self.0.admit(&keys, None, now)
    }
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;
    use std::time::Instant;

    use super::LoginLimiter;

    fn fail(limiter: &LoginLimiter, username: &str, address: &str) -> bool {
        let address: IpAddr = address.parse().unwrap();
        limiter.admit(username, address, Instant::now()).is_some()
    }

    #[test]
    fn five_failures_per_account_from_any_address() {
        let limiter = LoginLimiter::default();
        for i in 1..=5 {
            assert!(fail(&limiter, "adam", &format!("10.0.0.{i}")), "{i}");
        }
        assert!(!fail(&limiter, "adam", "10.0.0.6"));
        assert!(fail(&limiter, "eve", "10.0.0.6"));
    }

    #[test]
    fn five_failures_per_address_across_accounts() {
        let limiter = LoginLimiter::default();
        for i in 1..=5 {
            assert!(fail(&limiter, &format!("user-{i}"), "10.0.0.1"), "{i}");
        }
        assert!(!fail(&limiter, "user-6", "10.0.0.1"));
        assert!(fail(&limiter, "user-6", "10.0.0.2"));
    }

    #[test]
    fn an_ipv6_network_counts_as_one_address() {
        let limiter = LoginLimiter::default();
        for i in 1..=5 {
            assert!(
                fail(&limiter, &format!("user-{i}"), &format!("2001:db8::{i}")),
                "{i}"
            );
        }
        assert!(!fail(&limiter, "user-6", "2001:db8::ffff"));
        assert!(fail(&limiter, "user-6", "2001:db8:0:1::1"));
    }

    #[test]
    fn long_usernames_count_only_per_address() {
        let limiter = LoginLimiter::default();
        let (long, longest_counted) = ("x".repeat(65), "x".repeat(64));
        for i in 1..=5 {
            assert!(fail(&limiter, &long, &format!("10.0.1.{i}")), "{i}");
            assert!(
                fail(&limiter, &longest_counted, &format!("10.0.2.{i}")),
                "{i}"
            );
        }
        assert!(fail(&limiter, &long, "10.0.1.6"));
        assert!(!fail(&limiter, &longest_counted, "10.0.2.6"));
    }
}
