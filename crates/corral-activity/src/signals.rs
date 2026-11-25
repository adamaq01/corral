use std::time::Duration;

use chrono::Utc;
use corral_core::domain::SandboxId;
use corral_core::ports::{ActivitySignals, Usage};

use crate::ActivityLock;
use crate::lock::Seen;

/// When what a sandbox used over an interval is work: either above its threshold.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Busy {
    /// Processor time, as a percentage of one core.
    pub cpu_percent: f64,
    pub network_bytes_per_sec: u64,
}

impl Busy {
    #[must_use]
    pub fn of(&self, usage: &Usage) -> bool {
        if usage.interval.is_zero() {
            return false;
        }
        let cpu = usage.cpu.as_secs_f64() / usage.interval.as_secs_f64() * 100.0;
        let network = u128::from(usage.network_bytes) * 1000;
        cpu >= self.cpu_percent
            || network >= u128::from(self.network_bytes_per_sec) * usage.interval.as_millis()
    }
}

/// Takes the signals of work into what the idle sweep knows.
#[derive(Clone)]
pub struct Signals {
    lock: ActivityLock,
    busy: Busy,
    report_every: Duration,
}

impl Signals {
    #[must_use]
    pub fn new(lock: ActivityLock, busy: Busy, report_every: Duration) -> Self {
        Self {
            lock,
            busy,
            report_every,
        }
    }
}

impl ActivitySignals for Signals {
    fn worked(&self, sandboxes: &[SandboxId]) {
        let now = Utc::now();
        let mut seen = self.lock.seen();
        for id in sandboxes {
            seen.entry(id.clone())
                .or_insert_with(|| Seen::since(now))
                .last_work = now;
        }
    }

    fn measured(&self, sandbox: &SandboxId, usage: Usage) -> Duration {
        let now = Utc::now();
        let mut seen = self.lock.seen();
        let entry = seen
            .entry(sandbox.clone())
            .or_insert_with(|| Seen::since(now));
        entry.reported = Some(now);
        if self.busy.of(&usage) {
            entry.last_work = now;
        }
        self.report_every
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUSY: Busy = Busy {
        cpu_percent: 5.0,
        network_bytes_per_sec: 4096,
    };

    fn usage(interval_secs: u64, cpu_millis: u64, network_bytes: u64) -> Usage {
        Usage {
            interval: Duration::from_secs(interval_secs),
            cpu: Duration::from_millis(cpu_millis),
            network_bytes,
        }
    }

    #[test]
    fn either_threshold_makes_an_interval_busy() {
        assert!(!BUSY.of(&usage(60, 2_999, 245_759)));
        assert!(BUSY.of(&usage(60, 3_000, 0)), "5% of a core");
        assert!(BUSY.of(&usage(60, 0, 245_760)), "4 KiB/s");
        assert!(BUSY.of(&usage(10, 60_000, 0)), "six cores");
    }

    #[test]
    fn a_report_over_no_time_says_only_that_the_sandbox_reports() {
        assert!(!BUSY.of(&usage(0, 5_000, 1 << 20)));
    }
}
