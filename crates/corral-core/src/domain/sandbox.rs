use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Serialize;
use ts_rs::TS;
use uuid::Uuid;

use super::{Handle, ImageKey, ImageReference, SandboxName, SizeKey, UserId};
use crate::Result;

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SandboxId(pub String);

/// The most an id generated now takes, so that each object an engine names after it fits a
/// Kubernetes name, such as the claim `home-<id>`, and a label value.
const ID_MAX: usize = 52;
const ID_HEX: usize = 6;

impl SandboxId {
    /// `<handle>-<name>-<6 hex>`, cut to fit, so the names of what an engine makes for the
    /// sandbox say whose it is. Sandboxes made earlier keep their `sbx-<12 hex>` id.
    #[must_use]
    pub fn generate(owner: &Handle, name: &SandboxName) -> Self {
        let hex = &Uuid::new_v4().simple().to_string()[..ID_HEX];
        let room = ID_MAX - ID_HEX - 2;
        let name = name.as_str().trim_matches('-');
        let owner = owner.as_str().trim_matches('-');
        let owner = cut(owner, room.saturating_sub(name.len()).max(room / 2));
        let name = cut(name, room - owner.len());
        let id = [owner, name, hex]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("-");
        // A Kubernetes Service, which agent-sandbox names after the Sandbox, starts with a letter.
        if id.starts_with(|c: char| c.is_ascii_lowercase()) {
            Self(id)
        } else {
            Self(format!("sbx-{id}"))
        }
    }
}

fn cut(slug: &str, max: usize) -> &str {
    slug[..slug.len().min(max)].trim_end_matches('-')
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
    /// How long it stays awake without work, if its owner chose.
    pub idle_timeout: Option<Duration>,
    /// Until when it stays awake with or without work, if its owner asked.
    pub keep_awake_until: Option<DateTime<Utc>>,
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

    use super::{Handle, SandboxId, SandboxImage, SandboxName, SandboxStatus};

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
    fn generated_ids_name_the_owner_and_the_sandbox_and_fit_kubernetes() {
        let generate = |owner: &str, name: &str| {
            SandboxId::generate(
                &Handle::parse(owner).unwrap(),
                &SandboxName::parse(name).unwrap(),
            )
            .0
        };
        let long = "a".repeat(32);
        for (owner, name, prefix) in [
            ("adam", "main", "adam-main-".to_owned()),
            ("-adam-", "-main-", "adam-main-".to_owned()),
            ("1adam", "main", "sbx-1adam-main-".to_owned()),
            (&long, "main", format!("{long}-main-")),
            ("adam", &long, format!("adam-{}-", "a".repeat(32))),
            (&long, &long, format!("{0}-{0}-", "a".repeat(22))),
            (
                "ab-",
                &format!("-{}", "b".repeat(31)),
                format!("ab-{}-", "b".repeat(31)),
            ),
        ] {
            let id = generate(owner, name);
            let hex = id.strip_prefix(&prefix).unwrap_or_else(|| panic!("{id}"));
            assert_eq!(hex.len(), 6, "{id}");
            assert!(
                hex.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')),
                "{id}"
            );
            assert!(id.len() <= 56 && !id.contains("--"), "{id}");
        }
        assert_ne!(generate("adam", "main"), generate("adam", "main"));
    }
}
