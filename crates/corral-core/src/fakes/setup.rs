use std::collections::BTreeSet;
use std::sync::Mutex;

use async_trait::async_trait;

use super::{entry, file, lock, variable};
use crate::app::NewSharedEntry;
use crate::domain::{Attachment, Entry, EntryId, Owner, SandboxId, Shared, UserId};
use crate::ports::SetupRepo;
use crate::{Error, Result};

#[derive(Default)]
pub struct MemorySetupRepo {
    entries: Mutex<Vec<Entry>>,
    attachments: Mutex<Vec<Attachment>>,
    awaiting: Mutex<BTreeSet<UserId>>,
}

impl MemorySetupRepo {
    /// Marks the user as having kept credentials before entries existed.
    pub fn await_adoption(&self, user: UserId) {
        lock(&self.awaiting).insert(user);
    }

    fn entries_where(&self, keep: impl Fn(&Entry) -> bool) -> Vec<Entry> {
        lock(&self.entries)
            .iter()
            .filter(|entry| keep(entry))
            .cloned()
            .collect()
    }
}

/// A plain shared variable that applies to new sandboxes.
#[must_use]
pub fn shared_variable(name: &str, value: &str, shared: Shared) -> NewSharedEntry {
    NewSharedEntry {
        shared,
        entry: entry(variable(name), value),
    }
}

/// A plain shared file that applies to new sandboxes.
#[must_use]
pub fn shared_file(path: &str, text: &str, shared: Shared) -> NewSharedEntry {
    NewSharedEntry {
        shared,
        entry: entry(file(path), text),
    }
}

#[async_trait]
impl SetupRepo for MemorySetupRepo {
    async fn entries(&self, owner: UserId) -> Result<Vec<Entry>> {
        Ok(self.entries_where(|entry| entry.owner == Owner::Member(owner)))
    }

    async fn shared_entries(&self) -> Result<Vec<Entry>> {
        Ok(self.entries_where(|entry| entry.owner.shared().is_some()))
    }

    async fn every_member_entry(&self) -> Result<Vec<Entry>> {
        Ok(self.entries_where(|entry| entry.owner.member().is_some()))
    }

    async fn insert(&self, entry: &Entry) -> Result<()> {
        let mut entries = lock(&self.entries);
        if entries.iter().any(|held| held.id == entry.id) {
            return Err(Error::Conflict("the entry already exists".to_owned()));
        }
        entries.push(entry.clone());
        Ok(())
    }

    async fn update(&self, entry: &Entry, overrides: &[Attachment]) -> Result<bool> {
        let mut entries = lock(&self.entries);
        let Some(held) = entries
            .iter_mut()
            .find(|held| held.id == entry.id && held.owner.member() == entry.owner.member())
        else {
            return Ok(false);
        };
        *held = Entry {
            id: held.id,
            created_at: held.created_at,
            ..entry.clone()
        };
        let mut attachments = lock(&self.attachments);
        for attachment in overrides {
            if let Some(held) = attachments
                .iter_mut()
                .find(|held| held.sandbox == attachment.sandbox && held.entry == attachment.entry)
            {
                held.overriding.clone_from(&attachment.overriding);
            }
        }
        Ok(true)
    }

    async fn delete(&self, owner: &Owner, id: EntryId) -> Result<bool> {
        let mut entries = lock(&self.entries);
        let before = entries.len();
        entries.retain(|entry| !(entry.owner == *owner && entry.id == id));
        let deleted = entries.len() < before;
        if deleted {
            lock(&self.attachments).retain(|attachment| attachment.entry != id);
        }
        Ok(deleted)
    }

    async fn attachments(&self, owner: UserId) -> Result<Vec<Attachment>> {
        let theirs: BTreeSet<EntryId> = lock(&self.entries)
            .iter()
            .filter(|entry| entry.owner == Owner::Member(owner))
            .map(|entry| entry.id)
            .collect();
        Ok(lock(&self.attachments)
            .iter()
            .filter(|attachment| theirs.contains(&attachment.entry))
            .cloned()
            .collect())
    }

    async fn attachments_of(&self, entry: EntryId) -> Result<Vec<Attachment>> {
        Ok(lock(&self.attachments)
            .iter()
            .filter(|attachment| attachment.entry == entry)
            .cloned()
            .collect())
    }

    async fn attach(&self, attachment: &Attachment) -> Result<()> {
        if !lock(&self.entries)
            .iter()
            .any(|entry| entry.id == attachment.entry)
        {
            return Err(Error::NotFound("setup entry".to_owned()));
        }
        let mut attachments = lock(&self.attachments);
        attachments
            .retain(|held| !(held.sandbox == attachment.sandbox && held.entry == attachment.entry));
        attachments.push(attachment.clone());
        Ok(())
    }

    async fn detach(&self, sandbox: &SandboxId, entry: EntryId) -> Result<bool> {
        let mut attachments = lock(&self.attachments);
        let before = attachments.len();
        attachments.retain(|held| !(held.sandbox == *sandbox && held.entry == entry));
        Ok(attachments.len() < before)
    }

    async fn awaiting_adoption(&self) -> Result<Vec<UserId>> {
        Ok(lock(&self.awaiting).iter().copied().collect())
    }

    async fn adopted(&self, user: UserId) -> Result<()> {
        lock(&self.awaiting).remove(&user);
        Ok(())
    }
}
