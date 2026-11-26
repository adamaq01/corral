use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};

use super::Sandbox;
use crate::{Error, Result};

/// The shortest idle timeout an owner may choose.
pub const MIN_IDLE_TIMEOUT: Duration = Duration::from_mins(1);

/// How long sandboxes stay awake without work, within what the admin allows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IdleLimits {
    /// For a sandbox whose owner chose none.
    pub default_timeout: Duration,
    /// The longest an owner may choose.
    pub max_timeout: Duration,
    /// How far ahead an owner may keep a sandbox awake.
    pub max_keep_awake: Duration,
}

impl IdleLimits {
    /// How long `sandbox` stays awake without work: its owner's choice, never past the most
    /// allowed now, or the default.
    #[must_use]
    pub fn timeout_of(&self, sandbox: &Sandbox) -> Duration {
        sandbox
            .idle_timeout
            .unwrap_or(self.default_timeout)
            .min(self.max_timeout)
    }

    /// Refuses a timeout an owner may not choose.
    pub fn check_timeout(&self, timeout: Duration) -> Result<()> {
        if (MIN_IDLE_TIMEOUT..=self.max_timeout).contains(&timeout) {
            return Ok(());
        }
        Err(Error::Invalid(format!(
            "an idle timeout is between {} and {}",
            minutes(MIN_IDLE_TIMEOUT),
            minutes(self.max_timeout)
        )))
    }

    /// Refuses a time an owner may not keep a sandbox awake until.
    pub fn check_keep_awake(&self, until: DateTime<Utc>, now: DateTime<Utc>) -> Result<()> {
        let furthest = TimeDelta::from_std(self.max_keep_awake).unwrap_or(TimeDelta::MAX);
        if until > now && until - now <= furthest {
            return Ok(());
        }
        Err(Error::Invalid(format!(
            "a sandbox is kept awake until a time in the next {}",
            minutes(self.max_keep_awake)
        )))
    }
}

fn minutes(duration: Duration) -> String {
    let minutes = duration.as_secs() / 60;
    match (minutes / 60, minutes % 60) {
        (0, minutes) => format!("{minutes} min"),
        (hours, 0) => format!("{hours} h"),
        (hours, minutes) => format!("{hours} h {minutes} min"),
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use crate::domain::{ImageKey, SandboxId, SandboxImage, SandboxName, SizeKey, UserId};

    const LIMITS: IdleLimits = IdleLimits {
        default_timeout: Duration::from_mins(30),
        max_timeout: Duration::from_hours(4),
        max_keep_awake: Duration::from_hours(24),
    };

    fn sandbox(idle_timeout: Option<Duration>) -> Sandbox {
        Sandbox {
            id: SandboxId("adam-main-0a1b2c".to_owned()),
            owner: UserId(uuid::Uuid::nil()),
            name: SandboxName::parse("main").unwrap(),
            image: SandboxImage::Catalog(ImageKey::parse("default").unwrap()),
            size: SizeKey::parse("small").unwrap(),
            created_at: Utc::now(),
            started_at: None,
            setup_changed_at: None,
            delete_at: None,
            idle_timeout,
            keep_awake_until: None,
        }
    }

    #[test]
    fn a_sandbox_takes_its_owners_timeout_within_the_maximum_or_the_default() {
        assert_eq!(LIMITS.timeout_of(&sandbox(None)), Duration::from_mins(30));
        assert_eq!(
            LIMITS.timeout_of(&sandbox(Some(Duration::from_mins(5)))),
            Duration::from_mins(5)
        );
        assert_eq!(
            LIMITS.timeout_of(&sandbox(Some(Duration::from_hours(8)))),
            Duration::from_hours(4),
            "a maximum lowered since the owner chose"
        );
    }

    #[test]
    fn an_owner_chooses_a_timeout_between_a_minute_and_the_maximum() {
        for fine in [Duration::from_mins(1), Duration::from_hours(4)] {
            assert!(LIMITS.check_timeout(fine).is_ok(), "{fine:?}");
        }
        for wrong in [Duration::from_secs(59), Duration::from_secs(4 * 3600 + 1)] {
            let message = LIMITS.check_timeout(wrong).unwrap_err().to_string();
            assert!(message.contains("between 1 min and 4 h"), "{message}");
        }
    }

    #[test]
    fn a_sandbox_is_kept_awake_until_a_time_ahead_within_the_maximum() {
        let now = Utc.with_ymd_and_hms(2026, 11, 25, 12, 0, 0).unwrap();
        assert!(
            LIMITS
                .check_keep_awake(now + TimeDelta::hours(24), now)
                .is_ok()
        );
        for wrong in [now, now - TimeDelta::hours(1), now + TimeDelta::hours(25)] {
            let message = LIMITS.check_keep_awake(wrong, now).unwrap_err().to_string();
            assert!(message.contains("in the next 24 h"), "{message}");
        }
    }
}
