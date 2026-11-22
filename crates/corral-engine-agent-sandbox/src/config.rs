use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentSandboxConfig {
    pub namespace: String,
    pub service_account: String,
    pub storage_class: String,
    pub agent_image: String,
    pub max_running: usize,
    pub idle_timeout: Duration,
}
