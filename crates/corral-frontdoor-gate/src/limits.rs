use std::time::Duration;

/// What one connection, and all unauthenticated ones together, may hold of the gate.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// How long a connection has to authenticate.
    pub login_grace: Duration,
    /// Connections that have not authenticated yet, across the gate.
    pub unauthenticated: usize,
    pub auth_attempts: usize,
    /// How often a quiet client is asked whether it is still there, and how many unanswered
    /// asks end its connection.
    pub keepalive_interval: Duration,
    pub keepalive_max: usize,
    /// Channels open at once in one connection, those waiting for the sandbox included.
    pub channels: usize,
    /// Remote forwards one connection may hold.
    pub remote_forwards: usize,
    /// Bytes a connection's client may send ahead of what its sandbox has taken.
    pub backlog: usize,
    /// How long a connection waits for its sandbox to wake.
    pub wake_patience: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            login_grace: Duration::from_mins(1),
            unauthenticated: 64,
            auth_attempts: 6,
            keepalive_interval: Duration::from_secs(30),
            keepalive_max: 4,
            channels: 64,
            remote_forwards: 16,
            backlog: 8 << 20,
            wake_patience: Duration::from_mins(5),
        }
    }
}
