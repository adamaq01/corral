use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
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
    #[must_use]
    pub fn is_file(&self) -> bool {
        matches!(self, Self::File(_))
    }

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

/// How a shared entry reaches members' sandboxes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Ord, PartialOrd, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub enum Shared {
    /// In every sandbox, which members cannot detach, override or shadow.
    Required,
    /// Attached where members choose, and to new sandboxes when it applies to them.
    Offered,
}

/// Whose an entry is: one member's, or the admin's for everyone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    Member(UserId),
    Everyone(Shared),
}

impl Owner {
    #[must_use]
    pub fn member(&self) -> Option<UserId> {
        match self {
            Self::Member(id) => Some(*id),
            Self::Everyone(_) => None,
        }
    }

    #[must_use]
    pub fn shared(&self) -> Option<Shared> {
        match self {
            Self::Member(_) => None,
            Self::Everyone(shared) => Some(*shared),
        }
    }

    /// Who the secret store keeps the entry's secret values under, its overrides' included.
    #[must_use]
    pub fn secrets_owner(&self) -> UserId {
        self.member().unwrap_or(UserId::SHARED)
    }
}

/// A variable or a file, which sandboxes attached to it receive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: EntryId,
    pub owner: Owner,
    pub target: Target,
    /// For a file whose text is a template: each `{{ NAME }}` in it takes the value the variable
    /// `NAME` has in the sandbox.
    pub template: bool,
    /// For a file: whether it is written executable.
    pub executable: bool,
    /// Whether each new sandbox of its member, or of every member for a shared entry, is attached
    /// to it.
    pub apply_to_new: bool,
    /// Its value. An override is kept as secret as the entry.
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
