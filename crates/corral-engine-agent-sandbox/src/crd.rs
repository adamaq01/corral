use k8s_openapi::api::core::v1::{PersistentVolumeClaimTemplate, PodTemplateSpec};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Condition;
use kube::CustomResource;
use serde::{Deserialize, Serialize};

#[derive(CustomResource, Clone, Debug, Deserialize, Serialize)]
#[kube(
    group = "agents.x-k8s.io",
    version = "v1beta1",
    kind = "Sandbox",
    namespaced,
    status = "SandboxStatus",
    schema = "disabled"
)]
#[serde(rename_all = "camelCase")]
pub struct SandboxSpec {
    #[serde(default)]
    pub operating_mode: OperatingMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed_pod_policy: Option<FailedPodPolicy>,
    pub pod_template: PodTemplateSpec,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub volume_claim_templates: Vec<PersistentVolumeClaimTemplate>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub enum OperatingMode {
    #[default]
    Running,
    Suspended,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub enum FailedPodPolicy {
    Retain,
    Recreate,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct SandboxStatus {
    #[serde(default)]
    pub conditions: Vec<Condition>,
}
