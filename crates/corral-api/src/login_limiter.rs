use std::net::{IpAddr, Ipv6Addr};
use std::num::NonZeroU32;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use corral_core::domain::Handle;
use governor::{DefaultKeyedRateLimiter, Quota};

const MAX_ATTEMPTS: NonZeroU32 = NonZeroU32::new(5).unwrap();
const WINDOW: Duration = Duration::from_mins(15);
const SWEEP_EVERY: u32 = 1024;

pub(crate) struct LoginLimiter {
    by_address: DefaultKeyedRateLimiter<IpAddr>,
    by_username: DefaultKeyedRateLimiter<Handle>,
    attempts: AtomicU32,
}

impl Default for LoginLimiter {
    fn default() -> Self {
        let quota = Quota::with_period(WINDOW)
            .expect("the window is not zero")
            .allow_burst(MAX_ATTEMPTS);
        Self {
            by_address: DefaultKeyedRateLimiter::keyed(quota),
            by_username: DefaultKeyedRateLimiter::keyed(quota),
            attempts: AtomicU32::new(0),
        }
    }
}

impl LoginLimiter {
    // Spends a cell before the password is checked: governor has no way to look without
    // spending, and checking only after a failure would let a correct guess through.
    pub(crate) fn admit(&self, username: &str, address: IpAddr) -> bool {
        if self
            .attempts
            .fetch_add(1, Ordering::Relaxed)
            .is_multiple_of(SWEEP_EVERY)
        {
            self.by_address.retain_recent();
            self.by_username.retain_recent();
        }
        self.by_address.check_key(&network(address)).is_ok()
            && Handle::parse(username)
                .ok()
                .is_none_or(|username| self.by_username.check_key(&username).is_ok())
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

    use super::LoginLimiter;

    fn ip(value: &str) -> IpAddr {
        value.parse().unwrap()
    }

    #[test]
    fn five_attempts_per_username_from_any_address() {
        let limiter = LoginLimiter::default();
        for i in 1..=5 {
            assert!(limiter.admit("adam", ip(&format!("10.0.0.{i}"))), "{i}");
        }
        assert!(!limiter.admit("adam", ip("10.0.0.6")));
        assert!(limiter.admit("eve", ip("10.0.0.6")));
    }

    #[test]
    fn five_attempts_per_address_across_usernames() {
        let limiter = LoginLimiter::default();
        for i in 1..=5 {
            assert!(limiter.admit(&format!("user-{i}"), ip("10.0.0.1")), "{i}");
        }
        assert!(!limiter.admit("user-6", ip("10.0.0.1")));
        assert!(!limiter.admit("Not A Handle", ip("10.0.0.1")));
        assert!(limiter.admit("user-6", ip("10.0.0.2")));
    }

    #[test]
    fn ipv6_counts_per_64_and_mapped_ipv4_per_address() {
        let limiter = LoginLimiter::default();
        for i in 1..=5 {
            assert!(limiter.admit(&format!("v6-{i}"), ip(&format!("2001:db8::{i}"))));
            assert!(limiter.admit(&format!("v4-{i}"), ip("::ffff:192.0.2.1")));
        }
        assert!(!limiter.admit("v6-6", ip("2001:db8::ffff:6")));
        assert!(limiter.admit("v6-6", ip("2001:db8:0:1::6")));
        assert!(!limiter.admit("v4-6", ip("192.0.2.1")));
        assert!(limiter.admit("v4-6", ip("::ffff:192.0.2.2")));
    }

    #[test]
    fn usernames_that_are_not_handles_count_only_per_address() {
        let limiter = LoginLimiter::default();
        let long = "x".repeat(10_000);
        for i in 1..=6 {
            assert!(limiter.admit(&long, ip(&format!("10.0.1.{i}"))), "{i}");
        }
    }
}
