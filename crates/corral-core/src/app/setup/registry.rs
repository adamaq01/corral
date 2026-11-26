use crate::domain::{Attachment, Entry, EntryId, SandboxId, StoredValue};
use crate::{Error, Result};

/// One user's entries and where they are attached, loaded together so that a change can be
/// tried on a copy before it is saved.
#[derive(Clone, Debug)]
pub(super) struct Registry {
    /// Variables before files, each kind by name.
    pub entries: Vec<Entry>,
    pub attachments: Vec<Attachment>,
}

/// Whose entries a composition takes: one sandbox's, those a new sandbox gets, or those chosen
/// for a new sandbox.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Scope<'a> {
    Sandbox(&'a SandboxId),
    New,
    Chosen(&'a [EntryId]),
}

impl Registry {
    pub fn new(mut entries: Vec<Entry>, attachments: Vec<Attachment>) -> Self {
        entries
            .sort_by(|a, b| (&a.target, a.created_at, a.id).cmp(&(&b.target, b.created_at, b.id)));
        Self {
            entries,
            attachments,
        }
    }

    pub fn entry(&self, id: EntryId) -> Result<&Entry> {
        self.entries
            .iter()
            .find(|entry| entry.id == id)
            .ok_or_else(|| Error::NotFound(format!("setup entry {}", id.0)))
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

    /// What each entry of the scope gives it: its override there, or its own value.
    pub fn effective(&self, scope: Scope<'_>) -> Vec<(&Entry, &StoredValue)> {
        match scope {
            Scope::Sandbox(sandbox) => self
                .attached(sandbox)
                .into_iter()
                .map(|(entry, attachment)| {
                    (
                        entry,
                        attachment.overriding.as_ref().unwrap_or(&entry.value),
                    )
                })
                .collect(),
            Scope::New => self
                .entries
                .iter()
                .filter(|entry| entry.apply_to_new)
                .map(|entry| (entry, &entry.value))
                .collect(),
            Scope::Chosen(ids) => self
                .entries
                .iter()
                .filter(|entry| ids.contains(&entry.id))
                .map(|entry| (entry, &entry.value))
                .collect(),
        }
    }

    /// Whether an entry made earlier has the same target, as only entries made before targets
    /// were unique can.
    pub fn is_duplicate(&self, entry: &Entry) -> bool {
        self.entries.iter().any(|other| {
            other.target == entry.target
                && (other.created_at, other.id) < (entry.created_at, entry.id)
        })
    }

    pub fn put_entry(&mut self, entry: Entry) {
        match self.entries.iter_mut().find(|held| held.id == entry.id) {
            Some(held) => *held = entry,
            None => self.entries.push(entry),
        }
    }

    pub fn remove_entry(&mut self, id: EntryId) {
        self.entries.retain(|entry| entry.id != id);
        self.attachments.retain(|attachment| attachment.entry != id);
    }

    pub fn put_attachment(&mut self, attachment: Attachment) {
        self.remove_attachment(&attachment.sandbox.clone(), attachment.entry);
        self.attachments.push(attachment);
    }

    pub fn remove_attachment(&mut self, sandbox: &SandboxId, entry: EntryId) {
        self.attachments
            .retain(|held| !(held.sandbox == *sandbox && held.entry == entry));
    }
}
