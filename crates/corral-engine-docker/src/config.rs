use ssh_key::PublicKey;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DockerConfig {
    /// The network sandboxes join. Only sandboxes and the SSH front door may be on it, never
    /// Corral: its internal listener is unauthenticated and wakes any sandbox it is asked about.
    pub network: String,
    pub proxy_public_key: PublicKey,
    pub max_running: usize,
}
