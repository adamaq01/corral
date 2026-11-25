use std::sync::Mutex;
use std::time::Duration;

use super::lock;
use crate::domain::SandboxId;
use crate::ports::{ActivitySignals, Usage};

/// Keeps every signal it gets, and asks for a report every minute.
#[derive(Default)]
pub struct RecordedSignals {
    worked: Mutex<Vec<SandboxId>>,
    measured: Mutex<Vec<(SandboxId, Usage)>>,
}

impl RecordedSignals {
    pub const REPORT_EVERY: Duration = Duration::from_mins(1);

    #[must_use]
    pub fn seen_work(&self) -> Vec<SandboxId> {
        lock(&self.worked).clone()
    }

    #[must_use]
    pub fn reports(&self) -> Vec<(SandboxId, Usage)> {
        lock(&self.measured).clone()
    }
}

impl ActivitySignals for RecordedSignals {
    fn worked(&self, sandboxes: &[SandboxId]) {
        lock(&self.worked).extend_from_slice(sandboxes);
    }

    fn measured(&self, sandbox: &SandboxId, usage: Usage) -> Duration {
        lock(&self.measured).push((sandbox.clone(), usage));
        Self::REPORT_EVERY
    }
}
