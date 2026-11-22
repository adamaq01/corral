use serde::Deserialize;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DockerConfig {
    /// The network sandboxes join. Only sandboxes and the SSH front door may be on it, never
    /// Corral: its internal listener is unauthenticated and wakes any sandbox it is asked about.
    pub network: String,
}

impl Default for DockerConfig {
    fn default() -> Self {
        Self {
            network: "corral-sandboxes".to_owned(),
        }
    }
}
