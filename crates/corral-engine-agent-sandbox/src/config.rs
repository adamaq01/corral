use serde::Deserialize;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSandboxConfig {
    pub namespace: String,
    pub service_account: String,
    pub storage_class: String,
    pub agent_image: String,
}
