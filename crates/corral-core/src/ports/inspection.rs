use chrono::{DateTime, Utc};

/// What an engine reports about how a sandbox runs, for admins without access to the engine.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Inspection {
    pub facts: Vec<Fact>,
    /// The most recent first.
    pub events: Vec<EngineEvent>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fact {
    pub label: String,
    /// `None` for what does not exist right now, such as the pod of a sleeping sandbox.
    pub value: Option<String>,
}

impl Fact {
    pub fn new(label: &str, value: impl Into<Option<String>>) -> Self {
        Self {
            label: label.to_owned(),
            value: value.into(),
        }
    }

    /// A moment, in UTC to the second.
    pub fn at(label: &str, at: Option<DateTime<Utc>>) -> Self {
        Self::new(label, at.map(utc))
    }
}

/// How facts show a moment: in UTC, to the second.
#[must_use]
pub fn utc(at: DateTime<Utc>) -> String {
    at.format("%Y-%m-%d %H:%M:%S UTC").to_string()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    Normal,
    Warning,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineEvent {
    pub at: Option<DateTime<Utc>>,
    pub kind: EventKind,
    /// What the event is about, such as `Pod sbx-0123456789ab`.
    pub object: String,
    pub reason: String,
    pub message: String,
}
