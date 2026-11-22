use std::time::Duration;

use ssh_key::PublicKey;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DockerConfig {
    /// The network sandboxes join. Only sandboxes and the SSH front door may be on it, never
    /// Corral: its internal listener hands out the front door's key, which opens every sandbox.
    pub network: String,
    pub proxy_public_key: PublicKey,
    pub max_running: usize,
    pub idle_timeout: Duration,
}
