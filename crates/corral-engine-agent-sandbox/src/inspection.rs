use std::cmp::Reverse;

use chrono::{DateTime, Utc};
use corral_core::ports::{EngineEvent, EventKind, Fact, Inspection, utc};
use k8s_openapi::api::core::v1::{
    ContainerState, ContainerStatus, Event, PersistentVolumeClaim, Pod,
};
use k8s_openapi::jiff::Timestamp;

use crate::crd::Sandbox;
use crate::manifest::CONTAINER;

pub(crate) const RECENT_EVENTS: usize = 50;

/// What the cluster holds for one sandbox, each object `None` when it does not exist.
pub(crate) struct Observed {
    pub namespace: String,
    pub sandbox: Option<Sandbox>,
    pub pod: Option<Pod>,
    pub claim: Option<PersistentVolumeClaim>,
    /// About the Sandbox, its pod and its claim, in any order.
    pub events: Vec<Event>,
}

pub(crate) fn describe(observed: Observed) -> Inspection {
    let Observed {
        namespace,
        sandbox,
        pod,
        claim,
        events,
    } = observed;
    let status = pod.as_ref().and_then(|pod| pod.status.as_ref());
    let container = status.and_then(|status| {
        status
            .container_statuses
            .iter()
            .flatten()
            .find(|container| container.name == CONTAINER)
    });
    let mut facts = vec![
        Fact::new("Namespace", namespace),
        Fact::new(
            "Sandbox",
            sandbox
                .as_ref()
                .and_then(|sandbox| sandbox.metadata.name.clone()),
        ),
        Fact::new("Sandbox ready", sandbox.as_ref().and_then(sandbox_ready)),
        Fact::new(
            "Pod",
            pod.as_ref().and_then(|pod| pod.metadata.name.clone()),
        ),
        Fact::new(
            "Node",
            pod.as_ref()
                .and_then(|pod| pod.spec.as_ref()?.node_name.clone()),
        ),
        Fact::new("Phase", status.and_then(|status| status.phase.clone())),
        Fact::at(
            "Pod started",
            status.and_then(|status| moment(status.start_time.as_ref()?.0)),
        ),
        Fact::new(
            "Restarts",
            container.map(|container| container.restart_count.to_string()),
        ),
        Fact::new(
            "Container state",
            container.and_then(|container| state(container.state.as_ref()?)),
        ),
    ];
    if let Some(last) = container.and_then(|container| state(container.last_state.as_ref()?)) {
        facts.push(Fact::new("Last termination", last));
    }
    facts.extend([
        Fact::new("Image", image(sandbox.as_ref(), pod.as_ref())),
        Fact::new("Image digest", container.and_then(digest)),
        Fact::new(
            "Home volume claim",
            claim.as_ref().and_then(|claim| claim.metadata.name.clone()),
        ),
        Fact::new("Home volume size", claim.as_ref().and_then(claim_size)),
        Fact::new(
            "Storage class",
            claim
                .as_ref()
                .and_then(|claim| claim.spec.as_ref()?.storage_class_name.clone()),
        ),
        Fact::new(
            "Home volume",
            claim
                .as_ref()
                .and_then(|claim| claim.spec.as_ref()?.volume_name.clone()),
        ),
        Fact::new(
            "Home volume phase",
            claim
                .as_ref()
                .and_then(|claim| claim.status.as_ref()?.phase.clone()),
        ),
    ]);
    for condition in status
        .and_then(|status| status.conditions.as_ref())
        .into_iter()
        .flatten()
    {
        facts.push(Fact::new(
            &format!("Condition {}", condition.type_),
            described(
                &condition.status,
                [condition.reason.as_deref(), condition.message.as_deref()],
            ),
        ));
    }
    Inspection {
        facts,
        events: recent(events),
    }
}

fn sandbox_ready(sandbox: &Sandbox) -> Option<String> {
    let ready = sandbox
        .status
        .as_ref()?
        .conditions
        .iter()
        .find(|condition| condition.type_ == "Ready")?;
    Some(described(
        &ready.status,
        [Some(ready.reason.as_str()), Some(ready.message.as_str())],
    ))
}

fn state(state: &ContainerState) -> Option<String> {
    if let Some(running) = &state.running {
        return Some(
            match running.started_at.as_ref().and_then(|at| moment(at.0)) {
                Some(at) => format!("Running since {}", utc(at)),
                None => "Running".to_owned(),
            },
        );
    }
    if let Some(waiting) = &state.waiting {
        return Some(described(
            "Waiting",
            [waiting.reason.as_deref(), waiting.message.as_deref()],
        ));
    }
    let terminated = state.terminated.as_ref()?;
    let exit = format!("exit code {}", terminated.exit_code);
    let at = terminated
        .finished_at
        .as_ref()
        .and_then(|at| moment(at.0))
        .map(|at| format!("at {}", utc(at)));
    Some(described(
        "Terminated",
        [
            terminated.reason.as_deref(),
            terminated.message.as_deref(),
            Some(&exit),
            at.as_deref(),
        ],
    ))
}

/// `<state>: <detail>, <detail>`, without the details that are missing or empty.
fn described<'a>(state: &str, details: impl IntoIterator<Item = Option<&'a str>>) -> String {
    let details: Vec<&str> = details
        .into_iter()
        .flatten()
        .filter(|detail| !detail.is_empty())
        .collect();
    if details.is_empty() {
        state.to_owned()
    } else {
        format!("{state}: {}", details.join(", "))
    }
}

fn image(sandbox: Option<&Sandbox>, pod: Option<&Pod>) -> Option<String> {
    let running = pod.and_then(|pod| pod.spec.as_ref());
    let template = sandbox.and_then(|sandbox| sandbox.spec.pod_template.spec.as_ref());
    running
        .or(template)?
        .containers
        .iter()
        .find(|container| container.name == CONTAINER)?
        .image
        .clone()
}

// The runtime reports the image it resolved, as `<repository>@<digest>` or, for an image loaded
// into the node without a registry, only its id.
fn digest(container: &ContainerStatus) -> Option<String> {
    let resolved = container.image_id.as_str();
    let digest = resolved
        .rsplit_once('@')
        .map_or(resolved, |(_, digest)| digest);
    (!digest.is_empty()).then(|| digest.to_owned())
}

fn claim_size(claim: &PersistentVolumeClaim) -> Option<String> {
    let capacity = claim.status.as_ref()?.capacity.as_ref();
    let requested = claim.spec.as_ref()?.resources.as_ref()?.requests.as_ref();
    capacity
        .and_then(|capacity| capacity.get("storage"))
        .or_else(|| requested?.get("storage"))
        .map(|quantity| binary_units(&quantity.0))
}

// Corral asks for the home in bytes, which a provisioner may echo back as its capacity.
fn binary_units(quantity: &str) -> String {
    let Ok(bytes) = quantity.parse::<u64>() else {
        return quantity.to_owned();
    };
    let unit = ["Ti", "Gi", "Mi", "Ki"]
        .into_iter()
        .zip([40, 30, 20, 10])
        .find(|(_, shift)| bytes != 0 && bytes % (1 << shift) == 0);
    match unit {
        Some((unit, shift)) => format!("{}{unit}", bytes >> shift),
        None => quantity.to_owned(),
    }
}

fn recent(events: Vec<Event>) -> Vec<EngineEvent> {
    let mut events: Vec<EngineEvent> = events.into_iter().map(event).collect();
    events.sort_by_key(|event| Reverse(event.at));
    events.truncate(RECENT_EVENTS);
    events
}

fn event(event: Event) -> EngineEvent {
    let at = event
        .series
        .as_ref()
        .and_then(|series| series.last_observed_time.as_ref())
        .map(|at| at.0)
        .or_else(|| event.last_timestamp.as_ref().map(|at| at.0))
        .or_else(|| event.event_time.as_ref().map(|at| at.0))
        .or_else(|| event.metadata.creation_timestamp.as_ref().map(|at| at.0))
        .and_then(moment);
    let involved = &event.involved_object;
    let object = [involved.kind.as_deref(), involved.name.as_deref()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ");
    let message = event.message.unwrap_or_default();
    let count = event
        .series
        .as_ref()
        .and_then(|series| series.count)
        .or(event.count)
        .unwrap_or(1);
    let message = if count > 1 {
        format!("{message} ({count} times)")
    } else {
        message
    };
    EngineEvent {
        at,
        kind: if event.type_.as_deref() == Some("Warning") {
            EventKind::Warning
        } else {
            EventKind::Normal
        },
        object,
        reason: event.reason.unwrap_or_default(),
        message,
    }
}

fn moment(at: Timestamp) -> Option<DateTime<Utc>> {
    DateTime::from_timestamp(at.as_second(), at.subsec_nanosecond().try_into().ok()?)
}

#[cfg(test)]
mod tests {
    use corral_core::ports::{EventKind, Fact};
    use serde_json::{Value, json};

    use super::{Observed, describe};

    const ID: &str = "sbx-0123456789ab";
    const CLAIM: &str = "home-sbx-0123456789ab";

    fn sandbox() -> Value {
        json!({
            "apiVersion": "agents.x-k8s.io/v1beta1",
            "kind": "Sandbox",
            "metadata": {"name": ID},
            "spec": {
                "operatingMode": "Suspended",
                "podTemplate": {"spec": {"containers": [
                    {"name": "sandbox", "image": "ghcr.io/adamaq01/corral-sandbox:1"},
                ]}},
            },
            "status": {"conditions": [{
                "type": "Ready",
                "status": "False",
                "reason": "SandboxSuspended",
                "message": "",
                "lastTransitionTime": "2026-11-23T09:00:00Z",
            }]},
        })
    }

    fn claim() -> Value {
        json!({
            "apiVersion": "v1",
            "kind": "PersistentVolumeClaim",
            "metadata": {"name": CLAIM},
            "spec": {
                "storageClassName": "longhorn",
                "volumeName": "pvc-6f1c7a8e",
                "resources": {"requests": {"storage": "10737418240"}},
            },
            "status": {"phase": "Bound", "capacity": {"storage": "10Gi"}},
        })
    }

    fn pod() -> Value {
        json!({
            "apiVersion": "v1",
            "kind": "Pod",
            "metadata": {"name": ID},
            "spec": {
                "nodeName": "node-1",
                "containers": [{"name": "sandbox", "image": "ghcr.io/adamaq01/corral-sandbox:2"}],
            },
            "status": {
                "phase": "Running",
                "startTime": "2026-11-23T10:00:00Z",
                "conditions": [
                    {"type": "PodScheduled", "status": "True"},
                    {"type": "Ready", "status": "False", "reason": "ContainersNotReady",
                     "message": "containers with unready status: [sandbox]"},
                ],
                "containerStatuses": [{
                    "name": "sandbox",
                    "image": "ghcr.io/adamaq01/corral-sandbox:2",
                    "imageID": "ghcr.io/adamaq01/corral-sandbox@sha256:abc123",
                    "ready": false,
                    "restartCount": 3,
                    "state": {"waiting": {"reason": "CrashLoopBackOff",
                                          "message": "back-off 40s restarting"}},
                    "lastState": {"terminated": {"reason": "OOMKilled", "exitCode": 137,
                                                 "finishedAt": "2026-11-23T10:05:00Z"}},
                }],
            },
        })
    }

    fn event(object: (&str, &str), kind: &str, reason: &str, at: &str, count: i32) -> Value {
        json!({
            "apiVersion": "v1",
            "kind": "Event",
            "metadata": {"name": format!("{}.{reason}", object.1)},
            "involvedObject": {"kind": object.0, "name": object.1},
            "type": kind,
            "reason": reason,
            "message": format!("{reason} message"),
            "lastTimestamp": at,
            "count": count,
        })
    }

    fn parse<T: serde::de::DeserializeOwned>(value: Option<Value>) -> Option<T> {
        value.map(|value| serde_json::from_value(value).unwrap())
    }

    fn observed(sandbox: Option<Value>, pod: Option<Value>, claim: Option<Value>) -> Observed {
        Observed {
            namespace: "corral-sandboxes".to_owned(),
            sandbox: parse(sandbox),
            pod: parse(pod),
            claim: parse(claim),
            events: Vec::new(),
        }
    }

    fn table(facts: &[Fact]) -> Vec<(&str, Option<&str>)> {
        facts
            .iter()
            .map(|fact| (fact.label.as_str(), fact.value.as_deref()))
            .collect()
    }

    #[test]
    fn a_sleeping_sandbox_has_its_home_but_no_pod() {
        let inspection = describe(observed(Some(sandbox()), None, Some(claim())));
        assert_eq!(
            table(&inspection.facts),
            [
                ("Namespace", Some("corral-sandboxes")),
                ("Sandbox", Some(ID)),
                ("Sandbox ready", Some("False: SandboxSuspended")),
                ("Pod", None),
                ("Node", None),
                ("Phase", None),
                ("Pod started", None),
                ("Restarts", None),
                ("Container state", None),
                ("Image", Some("ghcr.io/adamaq01/corral-sandbox:1")),
                ("Image digest", None),
                ("Home volume claim", Some(CLAIM)),
                ("Home volume size", Some("10Gi")),
                ("Storage class", Some("longhorn")),
                ("Home volume", Some("pvc-6f1c7a8e")),
                ("Home volume phase", Some("Bound")),
            ]
        );
        assert_eq!(inspection.events, []);
    }

    #[test]
    fn a_crashing_pod_shows_where_it_runs_and_why_it_restarts() {
        let inspection = describe(observed(Some(sandbox()), Some(pod()), Some(claim())));
        let facts = table(&inspection.facts);
        for expected in [
            ("Pod", Some(ID)),
            ("Node", Some("node-1")),
            ("Phase", Some("Running")),
            ("Pod started", Some("2026-11-23 10:00:00 UTC")),
            ("Restarts", Some("3")),
            (
                "Container state",
                Some("Waiting: CrashLoopBackOff, back-off 40s restarting"),
            ),
            (
                "Last termination",
                Some("Terminated: OOMKilled, exit code 137, at 2026-11-23 10:05:00 UTC"),
            ),
            ("Image", Some("ghcr.io/adamaq01/corral-sandbox:2")),
            ("Image digest", Some("sha256:abc123")),
            ("Condition PodScheduled", Some("True")),
            (
                "Condition Ready",
                Some("False: ContainersNotReady, containers with unready status: [sandbox]"),
            ),
        ] {
            assert!(facts.contains(&expected), "{expected:?} in {facts:?}");
        }
    }

    #[test]
    fn a_first_run_has_no_last_termination_and_sizes_read_in_binary_units() {
        let mut first = pod();
        first["status"]["containerStatuses"][0]["lastState"] = json!({});
        let mut pending = claim();
        pending["status"] = json!({"phase": "Pending"});
        let inspection = describe(observed(Some(sandbox()), Some(first), Some(pending)));
        let facts = table(&inspection.facts);
        assert!(
            facts.iter().all(|(label, _)| *label != "Last termination"),
            "{facts:?}"
        );
        assert!(
            facts.contains(&("Home volume size", Some("10Gi"))),
            "{facts:?}"
        );
        for (quantity, shown) in [
            ("1073741824", "1Gi"),
            ("3072", "3Ki"),
            ("1536", "1536"),
            ("1000", "1000"),
            ("0", "0"),
            ("20Gi", "20Gi"),
        ] {
            assert_eq!(super::binary_units(quantity), shown);
        }
    }

    #[test]
    fn nothing_at_all_is_a_list_of_missing_facts() {
        let inspection = describe(observed(None, None, None));
        let facts = table(&inspection.facts);
        assert_eq!(facts[0], ("Namespace", Some("corral-sandboxes")));
        assert!(
            facts[1..].iter().all(|(_, value)| value.is_none()),
            "{facts:?}"
        );
    }

    #[test]
    fn events_are_newest_first_with_their_object_and_repeats() {
        let mut observed = observed(None, None, None);
        observed.events = [
            event(
                ("Sandbox", ID),
                "Normal",
                "Created",
                "2026-11-23T10:00:00Z",
                1,
            ),
            event(("Pod", ID), "Warning", "BackOff", "2026-11-23T10:06:00Z", 5),
            event(
                ("PersistentVolumeClaim", CLAIM),
                "Normal",
                "Provisioned",
                "2026-11-23T10:01:00Z",
                1,
            ),
        ]
        .into_iter()
        .map(|event| serde_json::from_value(event).unwrap())
        .collect();
        let events: Vec<_> = describe(observed)
            .events
            .into_iter()
            .map(|event| {
                (
                    event.at.unwrap().to_rfc3339(),
                    event.kind,
                    event.object,
                    event.reason,
                    event.message,
                )
            })
            .collect();
        let at = |time: &str| format!("2026-11-23T{time}+00:00");
        let owned = |text: &str| text.to_owned();
        assert_eq!(
            events,
            [
                (
                    at("10:06:00"),
                    EventKind::Warning,
                    format!("Pod {ID}"),
                    owned("BackOff"),
                    owned("BackOff message (5 times)")
                ),
                (
                    at("10:01:00"),
                    EventKind::Normal,
                    format!("PersistentVolumeClaim {CLAIM}"),
                    owned("Provisioned"),
                    owned("Provisioned message")
                ),
                (
                    at("10:00:00"),
                    EventKind::Normal,
                    format!("Sandbox {ID}"),
                    owned("Created"),
                    owned("Created message")
                ),
            ]
        );
    }
}
