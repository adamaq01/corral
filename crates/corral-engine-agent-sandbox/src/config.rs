use serde::Deserialize;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSandboxConfig {
    pub namespace: String,
    pub service_account: String,
    /// Without it, home volumes use the cluster's default storage class.
    #[serde(default)]
    pub storage_class: Option<String>,
    pub agent_image: String,
    /// The Corral overlay every sandbox mounts at /opt/corral, as an image volume.
    #[serde(default = "overlay_image")]
    pub overlay_image: String,
}

fn overlay_image() -> String {
    "ghcr.io/adamaq01/corral-overlay:latest".to_owned()
}
