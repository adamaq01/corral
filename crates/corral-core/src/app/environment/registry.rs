use std::collections::BTreeSet;

use crate::domain::{Attachment, Entry, EntryId, SandboxId, StoredValue, Target, VariableName};
use crate::{Error, Result};

/// One user's entries and where they are attached, loaded together so that a change can be
/// tried on a copy before it is saved.
#[derive(Clone, Debug)]
pub(super) struct Registry {
    /// Variables before files, each kind by name.
    pub entries: Vec<Entry>,
    pub attachments: Vec<Attachment>,
}

impl Registry {
    pub fn new(mut entries: Vec<Entry>, attachments: Vec<Attachment>) -> Self {
        entries.sort_by(|a, b| (&a.target, a.created_at).cmp(&(&b.target, b.created_at)));
        Self {
            entries,
            attachments,
        }
    }

    pub fn entry(&self, id: EntryId) -> Result<&Entry> {
        self.entries
            .iter()
            .find(|entry| entry.id == id)
            .ok_or_else(|| Error::NotFound(format!("environment entry {}", id.0)))
    }

    pub fn attachment(&self, sandbox: &SandboxId, entry: EntryId) -> Option<&Attachment> {
        self.attachments
            .iter()
            .find(|attachment| attachment.sandbox == *sandbox && attachment.entry == entry)
    }

    pub fn attachments_of(&self, entry: EntryId) -> impl Iterator<Item = &Attachment> {
        self.attachments
            .iter()
            .filter(move |attachment| attachment.entry == entry)
    }

    /// The entries attached to the sandbox, in the registry's order, each with its attachment.
    pub fn attached(&self, sandbox: &SandboxId) -> Vec<(&Entry, &Attachment)> {
        self.entries
            .iter()
            .filter_map(|entry| Some((entry, self.attachment(sandbox, entry.id)?)))
            .collect()
    }

    /// What each entry attached to the sandbox gives it: its override there, or its own value.
    pub fn effective(&self, sandbox: &SandboxId) -> Vec<(&Entry, &StoredValue)> {
        self.attached(sandbox)
            .into_iter()
            .map(|(entry, attachment)| {
                (
                    entry,
                    attachment.overriding.as_ref().unwrap_or(&entry.value),
                )
            })
            .collect()
    }

    pub fn variables(&self) -> BTreeSet<&VariableName> {
        self.entries
            .iter()
            .filter_map(|entry| match &entry.target {
                Target::Variable(name) => Some(name),
                Target::File(_) => None,
            })
            .collect()
    }

    /// Refuses to let two entries that collide both apply to new sandboxes, which would attach
    /// both to each.
    pub fn check_applies_to_new(&self, target: &Target, except: Option<EntryId>) -> Result<()> {
        match self.entries.iter().find(|entry| {
            entry.apply_to_new && Some(entry.id) != except && entry.target.collides(target)
        }) {
            Some(other) => Err(Error::Conflict(format!(
                "another entry, {}, already applies to new sandboxes; only one of the two can",
                other.target
            ))),
            None => Ok(()),
        }
    }

    pub fn put_entry(&mut self, entry: Entry) {
        match self.entries.iter_mut().find(|held| held.id == entry.id) {
            Some(held) => *held = entry,
            None => self.entries.push(entry),
        }
    }

    pub fn put_attachment(&mut self, attachment: Attachment) {
        self.attachments
            .retain(|held| !(held.sandbox == attachment.sandbox && held.entry == attachment.entry));
        self.attachments.push(attachment);
    }
}
