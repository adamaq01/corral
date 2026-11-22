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
}
