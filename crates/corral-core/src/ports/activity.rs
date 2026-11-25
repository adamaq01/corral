use std::time::Duration;

use crate::domain::SandboxId;

/// What a sandbox used over an interval, as it measures itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Usage {
    pub interval: Duration,
    /// Processor time across all its processes.
    pub cpu: Duration,
    /// Bytes in and out over its network interfaces, loopback left out.
    pub network_bytes: u64,
}

/// Where the signals that a sandbox is at work go, for whatever puts idle sandboxes to sleep.
pub trait ActivitySignals: Send + Sync {
    /// A person or a client worked in these sandboxes just now, as the front door saw.
    fn worked(&self, sandboxes: &[SandboxId]);
    /// The sandbox reports what it used. Answers how long until it reports again.
    fn measured(&self, sandbox: &SandboxId, usage: Usage) -> Duration;
}
