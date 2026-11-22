use corral_core::domain::SandboxStatus;
use k8s_openapi::api::core::v1::Pod;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Condition;

use crate::crd::{OperatingMode, Sandbox};

const READY: &str = "Ready";
const FAILED_REASONS: [&str; 4] = [
    "PodFailed",
    "PodSucceeded",
    "InvalidConfiguration",
    "MultiplePods",
];

#[must_use]
pub fn map(sandbox: &Sandbox, pod: Option<&Pod>) -> SandboxStatus {
    if sandbox.metadata.deletion_timestamp.is_some() {
        return SandboxStatus::Deleting;
    }
    if sandbox.spec.operating_mode == OperatingMode::Suspended {
        return SandboxStatus::Sleeping;
    }
    match current_condition(sandbox, READY) {
        Some(condition) if FAILED_REASONS.contains(&condition.reason.as_str()) => {
            let detail = if condition.message.is_empty() {
                &condition.reason
            } else {
                &condition.message
            };
            SandboxStatus::Failed(detail.clone())
        }
        Some(condition) if condition.status == "True" && is_live(sandbox, pod) => {
            SandboxStatus::Running
        }
        _ => SandboxStatus::Starting,
    }
}

fn current_condition<'a>(sandbox: &'a Sandbox, kind: &str) -> Option<&'a Condition> {
    sandbox
        .status
        .as_ref()?
        .conditions
        .iter()
        .find(|condition| {
            condition.type_ == kind && condition.observed_generation == sandbox.metadata.generation
        })
}

// Ready lags a pod deletion until the controller observes it, so the pod itself is checked too.
fn is_live(sandbox: &Sandbox, pod: Option<&Pod>) -> bool {
    pod.is_some_and(|pod| {
        pod.metadata.deletion_timestamp.is_none()
            && pod.metadata.owner_references.iter().flatten().any(|owner| {
                owner.controller == Some(true) && Some(&owner.uid) == sandbox.metadata.uid.as_ref()
            })
    })
}

#[cfg(test)]
mod tests {
    use corral_core::domain::SandboxStatus;
    use k8s_openapi::api::core::v1::Pod;
    use serde_json::{Value, json};

    use super::map;
    use crate::crd::Sandbox;

    const UID: &str = "6f1c7a8e-0000-4000-8000-000000000001";

    fn sandbox(mode: &str, generation: i64, ready: Option<(&str, &str, i64)>) -> Value {
        let conditions: Vec<Value> = ready
            .into_iter()
            .map(|(status, reason, observed)| {
                json!({
                    "type": "Ready",
                    "status": status,
                    "reason": reason,
                    "message": format!("{reason} message"),
                    "observedGeneration": observed,
                    "lastTransitionTime": "2026-11-21T10:00:00Z",
                })
            })
            .collect();
        json!({
            "apiVersion": "agents.x-k8s.io/v1beta1",
            "kind": "Sandbox",
            "metadata": {"name": "sbx-0123456789ab", "uid": UID, "generation": generation},
            "spec": {"operatingMode": mode, "podTemplate": {"spec": {"containers": []}}},
            "status": {"conditions": conditions},
        })
    }

    fn pod(owner_uid: &str) -> Value {
        json!({
            "apiVersion": "v1",
            "kind": "Pod",
            "metadata": {
                "name": "sbx-0123456789ab",
                "ownerReferences": [{
                    "apiVersion": "agents.x-k8s.io/v1beta1",
                    "kind": "Sandbox",
                    "name": "sbx-0123456789ab",
                    "uid": owner_uid,
                    "controller": true,
                }],
            },
        })
    }

    fn ready() -> Value {
        sandbox("Running", 3, Some(("True", "DependenciesReady", 3)))
    }

    #[test]
    fn status_mapping_table() {
        let mut deleting = ready();
        deleting["metadata"]["deletionTimestamp"] = json!("2026-11-21T10:00:00Z");
        let mut terminating = pod(UID);
        terminating["metadata"]["deletionTimestamp"] = json!("2026-11-21T10:00:00Z");
        let cases = [
            (
                "deletion timestamp",
                deleting,
                Some(pod(UID)),
                SandboxStatus::Deleting,
            ),
            (
                "suspended",
                sandbox("Suspended", 2, Some(("False", "SandboxSuspended", 2))),
                None,
                SandboxStatus::Sleeping,
            ),
            (
                "suspend not yet observed",
                sandbox("Suspended", 2, Some(("True", "DependenciesReady", 1))),
                Some(pod(UID)),
                SandboxStatus::Sleeping,
            ),
            ("ready", ready(), Some(pod(UID)), SandboxStatus::Running),
            (
                "ready but pod terminating",
                ready(),
                Some(terminating),
                SandboxStatus::Starting,
            ),
            ("ready but pod gone", ready(), None, SandboxStatus::Starting),
            (
                "ready but pod owned by another object",
                ready(),
                Some(pod("6f1c7a8e-0000-4000-8000-000000000002")),
                SandboxStatus::Starting,
            ),
            (
                "pod failed",
                sandbox("Running", 1, Some(("False", "PodFailed", 1))),
                Some(pod(UID)),
                SandboxStatus::Failed("PodFailed message".to_owned()),
            ),
            (
                "invalid configuration",
                sandbox("Running", 1, Some(("False", "InvalidConfiguration", 1))),
                None,
                SandboxStatus::Failed("InvalidConfiguration message".to_owned()),
            ),
            (
                "pod not ready yet",
                sandbox("Running", 1, Some(("False", "DependenciesNotReady", 1))),
                Some(pod(UID)),
                SandboxStatus::Starting,
            ),
            (
                "resume not yet observed",
                sandbox("Running", 3, Some(("True", "DependenciesReady", 2))),
                Some(pod(UID)),
                SandboxStatus::Starting,
            ),
            (
                "no status yet",
                sandbox("Running", 1, None),
                None,
                SandboxStatus::Starting,
            ),
        ];
        for (case, sandbox, pod, expected) in cases {
            let sandbox: Sandbox = serde_json::from_value(sandbox).unwrap();
            let pod: Option<Pod> = pod.map(|pod| serde_json::from_value(pod).unwrap());
            assert_eq!(map(&sandbox, pod.as_ref()), expected, "{case}");
        }
    }

    #[test]
    fn a_failure_without_a_message_reports_its_reason() {
        let mut failed = sandbox("Running", 1, Some(("False", "PodFailed", 1)));
        failed["status"]["conditions"][0]["message"] = json!("");
        let sandbox: Sandbox = serde_json::from_value(failed).unwrap();
        assert_eq!(
            map(&sandbox, None),
            SandboxStatus::Failed("PodFailed".to_owned())
        );
    }
}
