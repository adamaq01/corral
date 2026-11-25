use serde::Deserialize;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSandboxConfig {
    pub namespace: String,
    pub service_account: String,
    /// Without it, home volumes use the cluster's default storage class.
    #[serde(default)]
    pub storage_class: Option<String>,
    /// The Corral overlay every sandbox mounts at /opt/corral, as an image volume.
    #[serde(default = "overlay_image")]
    pub overlay_image: String,
    /// Secrets in the namespace that sandbox pods pull their images and the overlay with.
    #[serde(default)]
    pub image_pull_secrets: Vec<String>,
}

fn overlay_image() -> String {
    "ghcr.io/adamaq01/corral-overlay:latest".to_owned()
}
