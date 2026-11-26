use async_trait::async_trait;

use crate::Result;
use crate::domain::{Attachment, Entry, EntryId, Owner, SandboxId, UserId};

/// Members' and shared entries, and the sandboxes they are attached to.
#[async_trait]
pub trait SetupRepo: Send + Sync {
    /// The member's own entries.
    async fn entries(&self, owner: UserId) -> Result<Vec<Entry>>;
    /// The entries the admin shares with everyone.
    async fn shared_entries(&self) -> Result<Vec<Entry>>;
    /// Every member's own entries.
    async fn every_member_entry(&self) -> Result<Vec<Entry>>;
    async fn insert(&self, entry: &Entry) -> Result<()>;
    /// Replaces the entry, all but its creation, and the overrides of `overrides`, all at once.
    /// A member's entry stays theirs and a shared one shared, though how it is shared may
    /// change. Whether the entry exists.
    async fn update(&self, entry: &Entry, overrides: &[Attachment]) -> Result<bool>;
    /// Deletes the entry with its attachments. Whether it existed.
    async fn delete(&self, owner: &Owner, id: EntryId) -> Result<bool>;
    /// Every attachment of the member's own entries.
    async fn attachments(&self, owner: UserId) -> Result<Vec<Attachment>>;
    /// Every attachment of the entry, to whichever sandbox.
    async fn attachments_of(&self, entry: EntryId) -> Result<Vec<Attachment>>;
    /// Attaches the entry to the sandbox, or replaces the override of the attachment there.
    async fn attach(&self, attachment: &Attachment) -> Result<()>;
    /// Whether the entry was attached to the sandbox.
    async fn detach(&self, sandbox: &SandboxId, entry: EntryId) -> Result<bool>;
    /// Users whose credentials from before entries existed core has yet to make entries of.
    async fn awaiting_adoption(&self) -> Result<Vec<UserId>>;
    /// Does nothing for a user not awaiting adoption.
    async fn adopted(&self, user: UserId) -> Result<()>;
}
