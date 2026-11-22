use chrono::{DateTime, Utc};
use serde::Serialize;

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

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", content = "detail", rename_all = "lowercase")]
pub enum SandboxStatus {
    Starting,
    Running,
    Sleeping,
    Failed(String),
    Deleting,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::SandboxStatus;

    #[test]
    fn status_serializes_its_state_and_any_failure_detail() {
        assert_eq!(
            serde_json::to_value(SandboxStatus::Running).unwrap(),
            json!({ "state": "running" })
        );
        assert_eq!(
            serde_json::to_value(SandboxStatus::Failed("ran out of memory".to_owned())).unwrap(),
            json!({ "state": "failed", "detail": "ran out of memory" })
        );
    }
}
