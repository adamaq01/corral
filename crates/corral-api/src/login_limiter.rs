use std::net::IpAddr;
use std::num::NonZeroUsize;
use std::time::{Duration, Instant};

use axum::http::StatusCode;
use corral_core::Error;
use corral_core::app::{Attempt, FailureWindow, Network};
use corral_core::ports::{Answer, Identity, IdentityProvider};
use secrecy::SecretString;

use crate::error::ApiError;

const MAX_FAILURES: NonZeroUsize = NonZeroUsize::new(5).unwrap();
const WINDOW: Duration = Duration::from_mins(15);
// Longer usernames count only against the address, so guesses cannot grow memory per byte.
const MAX_USERNAME_KEY: usize = 64;

#[derive(Clone, PartialEq, Eq, Hash)]
enum Key {
    Network(Network),
    Account { provider: String, username: String },
}

pub(crate) struct LoginLimiter(FailureWindow<Key>);

impl Default for LoginLimiter {
    fn default() -> Self {
        Self(FailureWindow::new(MAX_FAILURES, WINDOW))
    }
}

impl LoginLimiter {
    pub(crate) async fn verify(
        &self,
        provider: &dyn IdentityProvider,
        username: String,
        password: SecretString,
        address: IpAddr,
    ) -> Result<Identity, ApiError> {
        let attempt = self
            .admit(provider.id(), &username, address, Instant::now())
            .ok_or_else(|| {
                ApiError::new(
                    StatusCode::TOO_MANY_REQUESTS,
                    "too many failed logins, try again later",
                )
            })?;
        let verified = provider
            .verify(Answer::Password { username, password })
            .await;
        if !matches!(verified, Err(Error::Forbidden)) {
            attempt.release();
        }
        Ok(verified?)
    }

    fn admit(
        &self,
        provider: &str,
        username: &str,
        address: IpAddr,
        now: Instant,
    ) -> Option<Attempt<'_, Key>> {
        let mut keys = vec![Key::Network(Network::from(address))];
        if username.len() <= MAX_USERNAME_KEY {
            keys.push(Key::Account {
                provider: provider.to_owned(),
                username: username.to_owned(),
            });
        }
        self.0.admit(&keys, None, now)
    }
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;
    use std::time::Instant;

    use super::LoginLimiter;

    fn fail(limiter: &LoginLimiter, provider: &str, username: &str, address: &str) -> bool {
        let address: IpAddr = address.parse().unwrap();
        limiter
            .admit(provider, username, address, Instant::now())
            .is_some()
    }

    #[test]
    fn five_failures_per_account_from_any_address() {
        let limiter = LoginLimiter::default();
        for i in 1..=5 {
            assert!(
                fail(&limiter, "local", "adam", &format!("10.0.0.{i}")),
                "{i}"
            );
        }
        assert!(!fail(&limiter, "local", "adam", "10.0.0.6"));
        assert!(fail(&limiter, "local", "eve", "10.0.0.6"));
        assert!(fail(&limiter, "ldap", "adam", "10.0.0.6"));
    }

    #[test]
    fn five_failures_per_address_across_accounts() {
        let limiter = LoginLimiter::default();
        for i in 1..=5 {
            let provider = if i % 2 == 0 { "local" } else { "ldap" };
            assert!(
                fail(&limiter, provider, &format!("user-{i}"), "10.0.0.1"),
                "{i}"
            );
        }
        assert!(!fail(&limiter, "local", "user-6", "10.0.0.1"));
        assert!(fail(&limiter, "local", "user-6", "10.0.0.2"));
    }

    #[test]
    fn long_usernames_count_only_per_address() {
        let limiter = LoginLimiter::default();
        let (long, longest_counted) = ("x".repeat(65), "x".repeat(64));
        for i in 1..=5 {
            assert!(
                fail(&limiter, "local", &long, &format!("10.0.1.{i}")),
                "{i}"
            );
            assert!(
                fail(&limiter, "local", &longest_counted, &format!("10.0.2.{i}")),
                "{i}"
            );
        }
        assert!(fail(&limiter, "local", &long, "10.0.1.6"));
        assert!(!fail(&limiter, "local", &longest_counted, "10.0.2.6"));
    }
}
