use std::collections::HashMap;

use bollard::Docker;
use bollard::errors::Error as DockerError;
use bollard::models::{
    ContainerInspectResponse, ContainerStateStatusEnum as DockerState, EventMessage, ImageInspect,
    Volume,
};
use bollard::query_parameters::EventsOptions;
use chrono::{DateTime, Utc};
use corral_core::domain::SandboxId;
use corral_core::ports::{EngineEvent, EventKind, Fact, Inspection};
use corral_core::{Error, Result};
use futures_util::TryStreamExt;

use crate::container;
use crate::docker_error::is_not_found;

const RECENT_EVENTS: usize = 50;
const EVENTS: [&str; 6] = ["create", "start", "kill", "stop", "die", "oom"];

/// What the daemon holds for one sandbox, each object `None` when it does not exist.
pub(crate) struct Observed {
    pub container: Option<ContainerInspectResponse>,
    pub image: Option<ImageInspect>,
    pub volume: Option<Volume>,
    /// The oldest first, as the daemon sends them.
    pub events: Vec<EventMessage>,
}

pub(crate) async fn observe(docker: &Docker, id: &SandboxId) -> Result<Observed> {
    let name = container::name(id);
    let container = found(docker.inspect_container(&name, None).await, &name)?;
    let image = match container
        .as_ref()
        .and_then(|container| container.image.as_ref())
    {
        Some(image) => found(docker.inspect_image(image).await, image)?,
        None => None,
    };
    let volume_name = container::volume_name(id);
    let volume = found(docker.inspect_volume(&volume_name).await, &volume_name)?;
    Ok(Observed {
        events: events_of(docker, &name).await?,
        container,
        image,
        volume,
    })
}

fn found<T>(answer: Result<T, DockerError>, what: &str) -> Result<Option<T>> {
    match answer {
        Ok(found) => Ok(Some(found)),
        Err(err) if is_not_found(&err) => Ok(None),
        Err(err) => Err(Error::internal(format!("inspect {what}"), err)),
    }
}

// The daemon keeps only its latest events, of every container, so these are the recent ones.
async fn events_of(docker: &Docker, name: &str) -> Result<Vec<EventMessage>> {
    let now = Utc::now();
    let filters = HashMap::from([
        ("type".to_owned(), vec!["container".to_owned()]),
        ("container".to_owned(), vec![name.to_owned()]),
        ("event".to_owned(), EVENTS.map(str::to_owned).to_vec()),
    ]);
    let options = EventsOptions {
        until: Some(format!(
            "{}.{:09}",
            now.timestamp(),
            now.timestamp_subsec_nanos()
        )),
        filters: Some(filters),
        ..EventsOptions::default()
    };
    let events: Vec<EventMessage> = docker
        .events(Some(options))
        .try_collect()
        .await
        .map_err(|err| Error::internal(format!("list the events of {name}"), err))?;
    Ok(events
        .into_iter()
        .filter(|event| attribute(event, "name") == Some(name))
        .collect())
}

pub(crate) fn describe(observed: Observed) -> Inspection {
    let Observed {
        container,
        image,
        volume,
        events,
    } = observed;
    let state = container
        .as_ref()
        .and_then(|container| container.state.as_ref());
    let running = state.and_then(|state| state.status) == Some(DockerState::RUNNING);
    let finished = state
        .and_then(|state| moment(state.finished_at.as_deref()?))
        .filter(|_| !running);
    let mut facts = vec![
        Fact::new(
            "Container",
            container
                .as_ref()
                .and_then(|container| container.name.as_deref())
                .map(|name| name.trim_start_matches('/').to_owned()),
        ),
        Fact::new(
            "Container ID",
            container
                .as_ref()
                .and_then(|container| container.id.clone()),
        ),
        Fact::new(
            "State",
            state
                .and_then(|state| state.status)
                .map(|status| status.to_string()),
        ),
        Fact::at(
            "Started",
            state.and_then(|state| moment(state.started_at.as_deref()?)),
        ),
        Fact::at("Finished", finished),
        Fact::new(
            "Exit code",
            state.filter(|_| finished.is_some()).and_then(|state| {
                let code = state.exit_code?;
                Some(if state.oom_killed == Some(true) {
                    format!("{code} (out of memory)")
                } else {
                    code.to_string()
                })
            }),
        ),
    ];
    if let Some(error) = state
        .and_then(|state| state.error.clone())
        .filter(|error| !error.is_empty())
    {
        facts.push(Fact::new("Error", error));
    }
    facts.extend([
        Fact::new(
            "Restarts",
            container
                .as_ref()
                .and_then(|container| container.restart_count)
                .map(|count| count.to_string()),
        ),
        Fact::new(
            "Network",
            container
                .as_ref()
                .and_then(|container| container.host_config.as_ref()?.network_mode.clone()),
        ),
        Fact::new(
            "Image",
            container
                .as_ref()
                .and_then(|container| container.config.as_ref()?.image.clone()),
        ),
        Fact::new(
            "Image ID",
            container
                .as_ref()
                .and_then(|container| container.image.clone()),
        ),
        Fact::new("Image digest", image.as_ref().and_then(digest)),
        Fact::new(
            "Home volume",
            volume.as_ref().map(|volume| volume.name.clone()),
        ),
        Fact::new(
            "Volume driver",
            volume.as_ref().map(|volume| volume.driver.clone()),
        ),
    ]);
    Inspection {
        facts,
        events: recent(&events),
    }
}

fn digest(image: &ImageInspect) -> Option<String> {
    let reference = image.repo_digests.as_ref()?.first()?;
    let (_, digest) = reference.rsplit_once('@')?;
    Some(digest.to_owned())
}

// The daemon reports a moment that never happened as the zero time, year 1.
fn moment(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|at| at.with_timezone(&Utc))
        .filter(|at| at.timestamp() > 0)
}

fn recent(events: &[EventMessage]) -> Vec<EngineEvent> {
    // A container exits after a stop asks it to, and on its own only when something failed.
    let mut stopping = false;
    let mut recent = Vec::with_capacity(events.len());
    for message in events {
        let action = message.action.as_deref().unwrap_or_default();
        let (kind, reason, text) = match action {
            "create" => (
                EventKind::Normal,
                "Created",
                "Created the container".to_owned(),
            ),
            "start" => {
                stopping = false;
                (
                    EventKind::Normal,
                    "Started",
                    "Started the container".to_owned(),
                )
            }
            "kill" => {
                stopping = true;
                let signal = attribute(message, "signal").unwrap_or("?");
                (
                    EventKind::Normal,
                    "Killing",
                    format!("Sent signal {signal}"),
                )
            }
            "stop" => {
                stopping = true;
                (
                    EventKind::Normal,
                    "Stopped",
                    "Stopped the container".to_owned(),
                )
            }
            "die" => {
                let code = attribute(message, "exitCode").unwrap_or("?");
                let kind = if stopping || code == "0" {
                    EventKind::Normal
                } else {
                    EventKind::Warning
                };
                (kind, "Exited", format!("Exited with code {code}"))
            }
            "oom" => (
                EventKind::Warning,
                "OutOfMemory",
                "Ran out of memory".to_owned(),
            ),
            other => (EventKind::Normal, other, String::new()),
        };
        recent.push(EngineEvent {
            at: message.time_nano.map(DateTime::from_timestamp_nanos),
            kind,
            object: format!(
                "Container {}",
                attribute(message, "name").unwrap_or_default()
            ),
            reason: reason.to_owned(),
            message: text,
        });
    }
    recent.reverse();
    recent.truncate(RECENT_EVENTS);
    recent
}

fn attribute<'a>(event: &'a EventMessage, key: &str) -> Option<&'a str> {
    event
        .actor
        .as_ref()?
        .attributes
        .as_ref()?
        .get(key)
        .map(String::as_str)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use bollard::models::{
        ContainerConfig, ContainerInspectResponse, ContainerState, ContainerStateStatusEnum,
        EventActor, EventMessage, ImageInspect, Volume,
    };
    use corral_core::ports::{EventKind, Fact};

    use super::{Observed, describe};

    const NAME: &str = "corral-sbx-0123456789ab";

    fn table(facts: &[Fact]) -> Vec<(&str, Option<&str>)> {
        facts
            .iter()
            .map(|fact| (fact.label.as_str(), fact.value.as_deref()))
            .collect()
    }

    fn event(action: &str, second: i64, attributes: &[(&str, &str)]) -> EventMessage {
        let mut attributes: HashMap<String, String> = attributes
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect();
        attributes.insert("name".to_owned(), NAME.to_owned());
        EventMessage {
            action: Some(action.to_owned()),
            actor: Some(EventActor {
                id: Some("0123".to_owned()),
                attributes: Some(attributes),
            }),
            time_nano: Some(second * 1_000_000_000),
            ..EventMessage::default()
        }
    }

    #[test]
    fn a_container_that_ran_out_of_memory() {
        let container = ContainerInspectResponse {
            id: Some("0123".to_owned()),
            name: Some(format!("/{NAME}")),
            image: Some("sha256:f945".to_owned()),
            restart_count: Some(0),
            config: Some(ContainerConfig {
                image: Some("ghcr.io/adamaq01/corral-sandbox:1".to_owned()),
                ..ContainerConfig::default()
            }),
            state: Some(ContainerState {
                status: Some(ContainerStateStatusEnum::EXITED),
                oom_killed: Some(true),
                exit_code: Some(137),
                started_at: Some("2026-11-23T10:00:00.5Z".to_owned()),
                finished_at: Some("2026-11-23T10:05:00Z".to_owned()),
                error: Some(String::new()),
                ..ContainerState::default()
            }),
            ..ContainerInspectResponse::default()
        };
        let image = ImageInspect {
            repo_digests: Some(vec![
                "ghcr.io/adamaq01/corral-sandbox@sha256:abc123".to_owned(),
            ]),
            ..ImageInspect::default()
        };
        let volume = Volume {
            name: "corral-home-sbx-0123456789ab".to_owned(),
            driver: "local".to_owned(),
            ..Volume::default()
        };
        let inspection = describe(Observed {
            container: Some(container),
            image: Some(image),
            volume: Some(volume),
            events: Vec::new(),
        });
        assert_eq!(
            table(&inspection.facts),
            [
                ("Container", Some(NAME)),
                ("Container ID", Some("0123")),
                ("State", Some("exited")),
                ("Started", Some("2026-11-23 10:00:00 UTC")),
                ("Finished", Some("2026-11-23 10:05:00 UTC")),
                ("Exit code", Some("137 (out of memory)")),
                ("Restarts", Some("0")),
                ("Network", None),
                ("Image", Some("ghcr.io/adamaq01/corral-sandbox:1")),
                ("Image ID", Some("sha256:f945")),
                ("Image digest", Some("sha256:abc123")),
                ("Home volume", Some("corral-home-sbx-0123456789ab")),
                ("Volume driver", Some("local")),
            ]
        );
    }

    #[test]
    fn a_container_never_started_has_no_start_finish_or_exit_code() {
        let container = ContainerInspectResponse {
            state: Some(ContainerState {
                status: Some(ContainerStateStatusEnum::CREATED),
                exit_code: Some(0),
                started_at: Some("0001-01-01T00:00:00Z".to_owned()),
                finished_at: Some("0001-01-01T00:00:00Z".to_owned()),
                ..ContainerState::default()
            }),
            ..ContainerInspectResponse::default()
        };
        let inspection = describe(Observed {
            container: Some(container),
            image: None,
            volume: None,
            events: Vec::new(),
        });
        let facts = table(&inspection.facts);
        assert!(facts.contains(&("State", Some("created"))), "{facts:?}");
        for label in ["Started", "Finished", "Exit code"] {
            assert!(facts.contains(&(label, None)), "{label} in {facts:?}");
        }
    }

    #[test]
    fn nothing_at_all_is_a_list_of_missing_facts() {
        let inspection = describe(Observed {
            container: None,
            image: None,
            volume: None,
            events: Vec::new(),
        });
        assert!(
            inspection.facts.iter().all(|fact| fact.value.is_none()),
            "{:?}",
            inspection.facts
        );
        assert_eq!(inspection.events, []);
    }

    #[test]
    fn an_exit_after_a_stop_is_normal_and_one_on_its_own_a_warning() {
        let inspection = describe(Observed {
            container: None,
            image: None,
            volume: None,
            events: vec![
                event("create", 1, &[]),
                event("start", 2, &[]),
                event("kill", 3, &[("signal", "15")]),
                event("stop", 4, &[]),
                event("die", 4, &[("exitCode", "143")]),
                event("start", 5, &[]),
                event("oom", 6, &[]),
                event("die", 6, &[("exitCode", "137")]),
            ],
        });
        let events: Vec<_> = inspection
            .events
            .iter()
            .map(|event| {
                (
                    event.at.unwrap().timestamp(),
                    event.kind,
                    event.reason.as_str(),
                    event.message.as_str(),
                )
            })
            .collect();
        assert_eq!(
            events,
            [
                (6, EventKind::Warning, "Exited", "Exited with code 137"),
                (6, EventKind::Warning, "OutOfMemory", "Ran out of memory"),
                (5, EventKind::Normal, "Started", "Started the container"),
                (4, EventKind::Normal, "Exited", "Exited with code 143"),
                (4, EventKind::Normal, "Stopped", "Stopped the container"),
                (3, EventKind::Normal, "Killing", "Sent signal 15"),
                (2, EventKind::Normal, "Started", "Started the container"),
                (1, EventKind::Normal, "Created", "Created the container"),
            ]
        );
        assert!(
            inspection
                .events
                .iter()
                .all(|event| event.object == format!("Container {NAME}"))
        );
    }
}
