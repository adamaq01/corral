use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::{HomePath, SandboxId, StoredValue, UserId, VariableName};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EntryId(pub Uuid);

impl EntryId {
    #[must_use]
    pub fn generate() -> Self {
        Self(Uuid::new_v4())
    }
}

/// Where an entry lands in a sandbox: an environment variable of its sessions, or a file in its
/// home.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Target {
    Variable(VariableName),
    File(HomePath),
}

impl Target {
    /// Whether two entries landing here could not both be attached to one sandbox: the same
    /// variable, or the same file or one inside the other.
    #[must_use]
    pub fn collides(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Variable(a), Self::Variable(b)) => a == b,
            (Self::File(a), Self::File(b)) => a.overlaps(b),
            _ => false,
        }
    }
}

impl std::fmt::Display for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Variable(name) => write!(f, "variable {name}"),
            Self::File(path) => write!(f, "file {path}"),
        }
    }
}

/// One of a user's variables or files, which sandboxes they attach it to receive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: EntryId,
    pub owner: UserId,
    pub target: Target,
    /// For a file whose text is a template: each `{{ NAME }}` in it takes the value the variable
    /// `NAME` has in the sandbox.
    pub template: bool,
    /// Whether each new sandbox of the owner is attached to it.
    pub apply_to_new: bool,
    /// Its value, which is secret or not for good: an override is kept the same way.
    pub value: StoredValue,
    pub created_at: DateTime<Utc>,
}

impl Entry {
    #[must_use]
    pub fn is_secret(&self) -> bool {
        self.value.is_secret()
    }
}

/// An entry attached to a sandbox, with any value it has there in place of its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attachment {
    pub sandbox: SandboxId,
    pub entry: EntryId,
    pub overriding: Option<StoredValue>,
}

#[cfg(test)]
mod tests {
    use super::Target;
    use crate::domain::{HomePath, VariableName};

    fn file(path: &str) -> Target {
        Target::File(HomePath::parse(path).unwrap())
    }

    fn variable(name: &str) -> Target {
        Target::Variable(VariableName::parse(name).unwrap())
    }

    #[test]
    fn targets_collide_on_one_name_or_on_one_file_or_a_file_inside_another() {
        for (a, b) in [
            (variable("GH_TOKEN"), variable("GH_TOKEN")),
            (file(".gitconfig"), file(".gitconfig")),
            (file(".config"), file(".config/tool/config.toml")),
            (file(".config/tool/config.toml"), file(".config/tool")),
        ] {
            assert!(a.collides(&b) && b.collides(&a), "{a} {b}");
        }
        for (a, b) in [
            (variable("GH_TOKEN"), variable("GH_TOKENS")),
            (file(".config/tool"), file(".config/tools")),
            (file(".config/a"), file(".config/b")),
            (variable("NOTES"), file("NOTES")),
        ] {
            assert!(!a.collides(&b) && !b.collides(&a), "{a} {b}");
        }
    }
}
