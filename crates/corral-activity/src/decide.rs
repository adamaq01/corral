use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};

use crate::lock::Seen;

/// Why a running sandbox stays awake, or that it sleeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// It worked within its idle timeout.
    Working,
    /// Its owner keeps it awake.
    KeptAwake,
    /// It does not report what it uses, as one started before its overlay had the reporter, so
    /// whether it works is unknown.
    Unreported,
    Sleep,
}

/// What the sweep decides for a running sandbox. A sandbox whose last report is older than three
/// report intervals is as one that never reported.
pub(crate) fn decide(
    seen: &Seen,
    timeout: Duration,
    keep_awake_until: Option<DateTime<Utc>>,
    report_every: Duration,
    now: DateTime<Utc>,
) -> Verdict {
    if keep_awake_until.is_some_and(|until| until > now) {
        return Verdict::KeptAwake;
    }
    let stale = delta(report_every * 3);
    if seen.reported.is_none_or(|reported| now - reported > stale) {
        return Verdict::Unreported;
    }
    if now - seen.last_work >= delta(timeout) {
        Verdict::Sleep
    } else {
        Verdict::Working
    }
}

fn delta(duration: Duration) -> TimeDelta {
    TimeDelta::from_std(duration).unwrap_or(TimeDelta::MAX)
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    const REPORT_EVERY: Duration = Duration::from_mins(1);

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 25, 12, 0, 0).unwrap()
    }

    fn mins(minutes: i64) -> TimeDelta {
        TimeDelta::minutes(minutes)
    }

    fn seen(worked_ago: TimeDelta, reported_ago: Option<TimeDelta>) -> Seen {
        Seen {
            last_work: now() - worked_ago,
            reported: reported_ago.map(|ago| now() - ago),
            since: now() - worked_ago,
            warned: false,
        }
    }

    fn verdict(seen: &Seen, timeout_mins: u64, keep_awake_until: Option<DateTime<Utc>>) -> Verdict {
        decide(
            seen,
            Duration::from_mins(timeout_mins),
            keep_awake_until,
            REPORT_EVERY,
            now(),
        )
    }

    #[test]
    fn a_reporting_sandbox_sleeps_once_its_timeout_passes_without_work() {
        let reporting = Some(mins(1));
        assert_eq!(
            verdict(&seen(mins(29), reporting), 30, None),
            Verdict::Working
        );
        assert_eq!(
            verdict(&seen(mins(30), reporting), 30, None),
            Verdict::Sleep
        );
        assert_eq!(
            verdict(&seen(mins(31), reporting), 30, None),
            Verdict::Sleep
        );
    }

    #[test]
    fn each_sandbox_has_its_own_timeout() {
        let idle_for_20 = seen(mins(20), Some(mins(0)));
        assert_eq!(verdict(&idle_for_20, 15, None), Verdict::Sleep);
        assert_eq!(verdict(&idle_for_20, 60, None), Verdict::Working);
    }

    #[test]
    fn a_kept_awake_sandbox_stays_awake_until_the_time_passes() {
        let idle = seen(mins(120), Some(mins(0)));
        assert_eq!(
            verdict(&idle, 30, Some(now() + mins(1))),
            Verdict::KeptAwake
        );
        assert_eq!(verdict(&idle, 30, Some(now())), Verdict::Sleep);
        assert_eq!(verdict(&idle, 30, Some(now() - mins(5))), Verdict::Sleep);
    }

    #[test]
    fn a_sandbox_that_does_not_report_stays_awake() {
        let never = seen(mins(120), None);
        assert_eq!(verdict(&never, 30, None), Verdict::Unreported);
        let stopped_reporting = seen(mins(120), Some(mins(4)));
        assert_eq!(verdict(&stopped_reporting, 30, None), Verdict::Unreported);
        let late_once = seen(mins(120), Some(mins(3)));
        assert_eq!(verdict(&late_once, 30, None), Verdict::Sleep);
    }

    #[test]
    fn work_in_the_future_or_an_unrepresentable_timeout_keeps_awake() {
        let ahead = Seen {
            last_work: now() + mins(5),
            ..seen(mins(0), Some(mins(0)))
        };
        assert_eq!(verdict(&ahead, 0, None), Verdict::Working);
        let idle = seen(mins(24 * 60), Some(mins(0)));
        assert_eq!(
            decide(&idle, Duration::MAX, None, REPORT_EVERY, now()),
            Verdict::Working
        );
    }
}
