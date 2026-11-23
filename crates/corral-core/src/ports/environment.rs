use async_trait::async_trait;

use crate::Result;
use crate::domain::{Attachment, Entry, EntryId, SandboxId, UserId};

/// Users' entries and the sandboxes they are attached to.
#[async_trait]
pub trait EnvironmentRepo: Send + Sync {
    async fn entries(&self, owner: UserId) -> Result<Vec<Entry>>;
    async fn insert(&self, entry: &Entry) -> Result<()>;
    /// Replaces the entry's value and flags; its owner, target and creation stay. Whether the
    /// entry exists.
    async fn update(&self, entry: &Entry) -> Result<bool>;
    /// Deletes the entry with its attachments. Whether it existed.
    async fn delete(&self, owner: UserId, id: EntryId) -> Result<bool>;
    /// Every attachment of the owner's entries.
    async fn attachments(&self, owner: UserId) -> Result<Vec<Attachment>>;
    /// Attaches the entry to the sandbox, or replaces the override of the attachment there.
    async fn attach(&self, attachment: &Attachment) -> Result<()>;
    /// Whether the entry was attached to the sandbox.
    async fn detach(&self, sandbox: &SandboxId, entry: EntryId) -> Result<bool>;
    /// Users whose credentials from before entries existed core has yet to make entries of.
    async fn awaiting_adoption(&self) -> Result<Vec<UserId>>;
    /// Does nothing for a user not awaiting adoption.
    async fn adopted(&self, user: UserId) -> Result<()>;
}
