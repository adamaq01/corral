use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};

use crate::ActivityError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdleDecision {
    KeepAwake,
    Sleep,
}

#[must_use]
pub fn decide(
    any_active: Result<bool, ActivityError>,
    last_active: DateTime<Utc>,
    timeout: Duration,
    now: DateTime<Utc>,
) -> IdleDecision {
    let inactive = any_active.is_ok_and(|active| !active);
    let timed_out = TimeDelta::from_std(timeout).is_ok_and(|timeout| now - last_active >= timeout);
    if inactive && timed_out {
        IdleDecision::Sleep
    } else {
        IdleDecision::KeepAwake
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 21, 12, 0, 0).unwrap()
    }

    fn mins(minutes: u64) -> Duration {
        Duration::from_secs(minutes * 60)
    }

    fn long_ago() -> DateTime<Utc> {
        now() - mins(24 * 60)
    }

    fn err() -> ActivityError {
        ActivityError::Exec("pod exec failed".to_owned())
    }

    #[test]
    fn probe_error_keeps_awake() {
        assert_eq!(
            decide(Err(err()), long_ago(), mins(30), now()),
            IdleDecision::KeepAwake
        );
    }

    #[test]
    fn sleeps_only_after_timeout_without_activity() {
        assert_eq!(
            decide(Ok(false), now() - mins(29), mins(30), now()),
            IdleDecision::KeepAwake
        );
        assert_eq!(
            decide(Ok(false), now() - mins(30), mins(30), now()),
            IdleDecision::Sleep
        );
        assert_eq!(
            decide(Ok(false), now() - mins(31), mins(30), now()),
            IdleDecision::Sleep
        );
        assert_eq!(
            decide(Ok(true), long_ago(), mins(30), now()),
            IdleDecision::KeepAwake
        );
    }

    #[test]
    fn last_active_in_the_future_keeps_awake() {
        assert_eq!(
            decide(Ok(false), now() + mins(5), mins(0), now()),
            IdleDecision::KeepAwake
        );
    }

    #[test]
    fn unrepresentable_timeout_never_sleeps() {
        assert_eq!(
            decide(Ok(false), long_ago(), Duration::MAX, now()),
            IdleDecision::KeepAwake
        );
    }
}
