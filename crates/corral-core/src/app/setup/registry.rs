use super::not_found;
use crate::domain::{Attachment, Entry, EntryId, SandboxId, Shared, StoredValue};
use crate::{Error, Result};

/// One member's entries, every shared entry, and where they are attached to the member's
/// sandboxes, loaded together so that a change can be tried on a copy before it is saved.
#[derive(Clone, Debug)]
pub(super) struct Registry {
    /// The member's own, variables before files, each kind by name.
    pub entries: Vec<Entry>,
    /// Everyone's, in the same order.
    pub shared: Vec<Entry>,
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
    pub fn new(
        mut entries: Vec<Entry>,
        mut shared: Vec<Entry>,
        attachments: Vec<Attachment>,
    ) -> Self {
        by_target(&mut entries);
        by_target(&mut shared);
        Self {
            entries,
            shared,
            attachments,
        }
    }

    /// A shared entry or one of the member's.
    pub fn entry(&self, id: EntryId) -> Result<&Entry> {
        self.shared
            .iter()
            .chain(&self.entries)
            .find(|entry| entry.id == id)
            .ok_or_else(|| not_found(id))
    }

    /// One of the member's own entries.
    pub fn own(&self, id: EntryId) -> Result<&Entry> {
        self.entries
            .iter()
            .find(|entry| entry.id == id)
            .ok_or_else(|| not_found(id))
    }

    /// An entry the member may attach, detach and override: any but a required one.
    pub fn attachable(&self, id: EntryId) -> Result<&Entry> {
        let entry = self.entry(id)?;
        if entry.owner.shared() == Some(Shared::Required) {
            return Err(Error::Invalid(format!(
                "{} is required in every sandbox",
                entry.target
            )));
        }
        Ok(entry)
    }

    pub fn required(&self) -> impl Iterator<Item = &Entry> {
        self.shared_as(Shared::Required)
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

    /// The entries attached to the sandbox, shared ones first, each with its attachment.
    pub fn attached(&self, sandbox: &SandboxId) -> Vec<(&Entry, &Attachment)> {
        self.shared
            .iter()
            .chain(&self.entries)
            .filter_map(|entry| Some((entry, self.attachment(sandbox, entry.id)?)))
            .collect()
    }

    /// What the scope gets, each entry with its value there. For one name or path, an
    /// override wins over its entry's own value, every required entry over the member's, and
    /// the member's over an offered one; within a layer, colliding entries are all kept, for
    /// the composition to refuse.
    pub fn effective(&self, scope: Scope<'_>) -> Vec<(&Entry, &StoredValue)> {
        let layers: [Vec<(&Entry, &StoredValue)>; 3] = [
            self.required().map(|entry| (entry, &entry.value)).collect(),
            self.entries
                .iter()
                .filter_map(|entry| Some((entry, self.value_in(scope, entry)?)))
                .collect(),
            self.shared_as(Shared::Offered)
                .filter_map(|entry| Some((entry, self.value_in(scope, entry)?)))
                .collect(),
        ];
        let mut effective: Vec<(&Entry, &StoredValue)> = Vec::new();
        for layer in layers {
            let kept: Vec<_> = layer
                .into_iter()
                .filter(|(entry, _)| {
                    !effective
                        .iter()
                        .any(|(above, _)| above.target.collides(&entry.target))
                })
                .collect();
            effective.extend(kept);
        }
        effective
    }

    /// Whether the member's entry takes the place of an offered one.
    pub fn shadows(&self, entry: &Entry) -> bool {
        entry.owner.member().is_some()
            && self
                .shared_as(Shared::Offered)
                .any(|shared| shared.target.collides(&entry.target))
    }

    /// Whether an entry of the member's takes the place of the offered one.
    pub fn shadowed(&self, shared: &Entry) -> bool {
        shared.owner.shared() == Some(Shared::Offered)
            && self
                .entries
                .iter()
                .any(|entry| entry.target.collides(&shared.target))
    }

    /// Whether an entry of the member's made earlier has the same target, as only entries made
    /// before targets were unique can.
    pub fn is_duplicate(&self, entry: &Entry) -> bool {
        entry.owner.member().is_some()
            && self.entries.iter().any(|other| {
                other.target == entry.target
                    && (other.created_at, other.id) < (entry.created_at, entry.id)
            })
    }

    pub fn put_entry(&mut self, entry: Entry) {
        let layer = match entry.owner.shared() {
            Some(_) => &mut self.shared,
            None => &mut self.entries,
        };
        match layer.iter_mut().find(|held| held.id == entry.id) {
            Some(held) => *held = entry,
            None => layer.push(entry),
        }
    }

    pub fn remove_entry(&mut self, id: EntryId) {
        self.entries.retain(|entry| entry.id != id);
        self.shared.retain(|entry| entry.id != id);
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

    /// The value the entry has in the scope, if the scope takes it.
    fn value_in<'r>(&'r self, scope: Scope<'_>, entry: &'r Entry) -> Option<&'r StoredValue> {
        match scope {
            Scope::Sandbox(sandbox) => self
                .attachment(sandbox, entry.id)
                .map(|attachment| attachment.overriding.as_ref().unwrap_or(&entry.value)),
            Scope::New => entry.apply_to_new.then_some(&entry.value),
            Scope::Chosen(ids) => ids.contains(&entry.id).then_some(&entry.value),
        }
    }

    fn shared_as(&self, kind: Shared) -> impl Iterator<Item = &Entry> {
        self.shared
            .iter()
            .filter(move |entry| entry.owner.shared() == Some(kind))
    }
}

/// Variables before files, each kind by name, then the oldest first.
pub(super) fn by_target(entries: &mut [Entry]) {
    entries.sort_by(|a, b| (&a.target, a.created_at, a.id).cmp(&(&b.target, b.created_at, b.id)));
}
