use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use super::{ImageKey, SandboxName, SizeKey, UserId};

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SandboxId(pub String);

impl SandboxId {
    #[must_use]
    pub fn generate() -> Self {
        Self(format!(
            "sbx-{}",
            &Uuid::new_v4().simple().to_string()[..12]
        ))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sandbox {
    pub id: SandboxId,
    pub owner: UserId,
    pub name: SandboxName,
    pub image: ImageKey,
    pub size: SizeKey,
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

    use super::{SandboxId, SandboxStatus};

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

    #[test]
    fn generated_ids_are_sbx_and_twelve_lowercase_hex() {
        let id = SandboxId::generate().0;
        let hex = id.strip_prefix("sbx-").unwrap();
        assert_eq!(hex.len(), 12, "{id}");
        assert!(
            hex.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')),
            "{id}"
        );
        assert_ne!(SandboxId::generate(), SandboxId::generate());
    }
}
