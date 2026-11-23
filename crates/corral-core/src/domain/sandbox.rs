use chrono::{DateTime, Utc};
use serde::Serialize;
use ts_rs::TS;
use uuid::Uuid;

use super::{ImageKey, ImageReference, SandboxName, SizeKey, UserId};
use crate::Result;

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

/// What a sandbox runs: a catalog image, by its key, or an image its owner named.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SandboxImage {
    Catalog(ImageKey),
    Named(ImageReference),
}

impl SandboxImage {
    /// Reads the text `Display` writes. A key never holds a `/` and a written-out reference
    /// always does.
    pub fn parse(text: &str) -> Result<Self> {
        if text.contains('/') {
            ImageReference::parse(text).map(Self::Named)
        } else {
            ImageKey::parse(text).map(Self::Catalog)
        }
    }
}

impl std::fmt::Display for SandboxImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Catalog(key) => key.fmt(f),
            Self::Named(reference) => reference.fmt(f),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sandbox {
    pub id: SandboxId,
    pub owner: UserId,
    pub name: SandboxName,
    pub image: SandboxImage,
    pub size: SizeKey,
    pub created_at: DateTime<Utc>,
    /// When Corral last started it from stopped, so with its environment of that moment.
    pub started_at: Option<DateTime<Utc>>,
    /// When a change last reached its environment: what it is attached to, or a value it gets.
    pub environment_changed_at: Option<DateTime<Utc>>,
    /// When it is to be deleted, if `corral delete` inside it scheduled that.
    pub delete_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
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

    use super::{SandboxId, SandboxImage, SandboxStatus};

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
    fn an_image_reads_back_from_its_text() {
        for text in [
            "ubuntu",
            "ghcr.io/acme/tools:1",
            "docker.io/library/debian:13",
        ] {
            assert_eq!(SandboxImage::parse(text).unwrap().to_string(), text);
        }
        assert!(matches!(
            SandboxImage::parse("ubuntu").unwrap(),
            SandboxImage::Catalog(_)
        ));
        assert!(matches!(
            SandboxImage::parse("debian/x").unwrap(),
            SandboxImage::Named(_)
        ));
        assert!(SandboxImage::parse("Ubuntu").is_err());
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
