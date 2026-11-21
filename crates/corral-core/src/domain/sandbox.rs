use chrono::{DateTime, Utc};

use super::{SandboxName, UserId};

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SandboxId(pub String);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sandbox {
    pub id: SandboxId,
    pub owner: UserId,
    pub name: SandboxName,
    pub image: String,
    pub size: String,
    pub is_default: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SandboxState {
    Starting,
    Running,
    Sleeping,
    Stopped,
    Failed,
    Deleting,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SandboxStatus {
    pub state: SandboxState,
    pub detail: Option<String>,
}
