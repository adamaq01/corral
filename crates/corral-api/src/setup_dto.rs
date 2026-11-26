use std::collections::BTreeSet;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use chrono::{DateTime, Utc};
use corral_core::app::{
    Attached, EntryChange, EntryView, NewEntry, NewSharedEntry, Preview, SharedEntryChange,
    SharedEntryView, Use,
};
use corral_core::domain::{
    Entry, Handle, HomePath, Kept, Owner, SandboxName, Shared, StoredValue, Target, VariableName,
};
use corral_core::{Error, Result};
use secrecy::{ExposeSecret, SecretSlice, SecretString};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub(crate) enum EntryKind {
    Variable,
    File,
}

/// A value as someone sends it: text, or any bytes in standard base64.
#[derive(Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ValueInput {
    Text(#[ts(type = "string")] SecretString),
    Base64(#[ts(type = "string")] SecretString),
}

impl EntryKind {
    /// The variable named `target`, or the file at the path `target` in the home.
    fn target(self, target: &str) -> Result<Target> {
        Ok(match self {
            Self::Variable => Target::Variable(VariableName::parse(target)?),
            Self::File => Target::File(HomePath::parse(target)?),
        })
    }
}

impl ValueInput {
    pub(crate) fn bytes(self) -> Result<SecretSlice<u8>> {
        match self {
            Self::Text(text) => Ok(SecretSlice::from(text.expose_secret().as_bytes().to_vec())),
            Self::Base64(encoded) => STANDARD
                .decode(encoded.expose_secret())
                .map(SecretSlice::from)
                .map_err(|_| Error::Invalid("the value is not standard base64".to_owned())),
        }
    }
}

/// A new variable, `NAME=value` in each session, or a file at a path inside the home.
#[derive(Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "NewEntry")]
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct NewEntryDto {
    kind: EntryKind,
    /// The variable's name, or the file's path relative to the home, which may start with `~/`.
    target: String,
    secret: bool,
    #[serde(default)]
    #[ts(optional, as = "Option<bool>")]
    template: bool,
    #[serde(default)]
    #[ts(optional, as = "Option<bool>")]
    executable: bool,
    apply_to_new: bool,
    value: ValueInput,
}

impl NewEntryDto {
    pub(crate) fn parse(self) -> Result<NewEntry> {
        Ok(NewEntry {
            target: self.kind.target(&self.target)?,
            secret: self.secret,
            template: self.template,
            executable: self.executable,
            apply_to_new: self.apply_to_new,
            value: self.value.bytes()?,
        })
    }
}

/// An entry the admin shares with everyone, as they first give it.
#[derive(Deserialize, TS)]
#[ts(rename = "NewSharedEntry")]
pub(crate) struct NewSharedEntryDto {
    shared: Shared,
    #[serde(flatten)]
    #[ts(flatten)]
    entry: NewEntryDto,
}

impl NewSharedEntryDto {
    pub(crate) fn parse(self) -> Result<NewSharedEntry> {
        Ok(NewSharedEntry {
            shared: self.shared,
            entry: self.entry.parse()?,
        })
    }
}

/// What an entry becomes, its attachments and overrides kept, and its value when given;
/// without one, the value stays.
#[derive(Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "EntryChange")]
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct EntryChangeDto {
    kind: EntryKind,
    target: String,
    secret: bool,
    template: bool,
    executable: bool,
    apply_to_new: bool,
    #[ts(optional)]
    value: Option<ValueInput>,
}

impl EntryChangeDto {
    pub(crate) fn parse(self) -> Result<EntryChange> {
        Ok(EntryChange {
            target: self.kind.target(&self.target)?,
            secret: self.secret,
            template: self.template,
            executable: self.executable,
            apply_to_new: self.apply_to_new,
            value: self.value.map(ValueInput::bytes).transpose()?,
        })
    }
}

/// What a shared entry becomes, how it is shared included.
#[derive(Deserialize, TS)]
#[ts(rename = "SharedEntryChange")]
pub(crate) struct SharedEntryChangeDto {
    shared: Shared,
    #[serde(flatten)]
    #[ts(flatten)]
    change: EntryChangeDto,
}

impl SharedEntryChangeDto {
    pub(crate) fn parse(self) -> Result<SharedEntryChange> {
        Ok(SharedEntryChange {
            shared: self.shared,
            change: self.change.parse()?,
        })
    }
}

/// What may be shown of a value: a plain one in full, a secret one only in part.
#[derive(Serialize, TS)]
#[serde(tag = "shown", rename_all = "lowercase")]
pub(crate) enum Shown {
    /// A plain value that is text.
    Text { text: String },
    /// A plain value that is not text, in standard base64.
    Binary { base64: String },
    /// A secret variable: its length in characters, with its first 4 and last 2 characters
    /// when it has 16 or more.
    Masked {
        length: usize,
        head: Option<String>,
        tail: Option<String>,
    },
    /// A secret file, or for a member a secret value the admin shares, of which only the size
    /// shows.
    Hidden,
}

#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "StoredValue")]
pub(crate) struct StoredValueDto {
    set_at: DateTime<Utc>,
    /// In bytes.
    size: usize,
    #[serde(flatten)]
    shown: Shown,
}

impl StoredValueDto {
    fn hidden(value: &StoredValue) -> Self {
        Self {
            set_at: value.set_at,
            size: value.size(),
            shown: Shown::Hidden,
        }
    }
}

impl From<&StoredValue> for StoredValueDto {
    fn from(value: &StoredValue) -> Self {
        let shown = match &value.kept {
            Kept::Plain(content) => match std::str::from_utf8(content) {
                Ok(text) => Shown::Text {
                    text: text.to_owned(),
                },
                Err(_) => Shown::Binary {
                    base64: STANDARD.encode(content),
                },
            },
            Kept::Secret(secret) => match &secret.glimpse {
                Some(glimpse) => {
                    let (head, tail) = glimpse.ends.clone().unzip();
                    Shown::Masked {
                        length: glimpse.length,
                        head,
                        tail,
                    }
                }
                None => Shown::Hidden,
            },
        };
        Self {
            set_at: value.set_at,
            size: value.size(),
            shown,
        }
    }
}

/// What a member and the admin alike see of an entry.
#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_excessive_bools)]
struct EntryFields {
    id: Uuid,
    kind: EntryKind,
    /// The variable's name, or the file's path relative to the home.
    target: String,
    secret: bool,
    /// For a text file: each `{{ NAME }}` takes the value of the variable `NAME` attached to
    /// the same sandbox.
    template: bool,
    /// For a file: whether it is written executable.
    executable: bool,
    /// Whether each new sandbox is attached to it.
    apply_to_new: bool,
    value: StoredValueDto,
    created_at: DateTime<Utc>,
    /// For a template: the variables its own value refers to.
    references: Vec<VariableName>,
}

impl EntryFields {
    fn new(entry: &Entry, value: StoredValueDto, references: BTreeSet<VariableName>) -> Self {
        let (kind, target) = match &entry.target {
            Target::Variable(name) => (EntryKind::Variable, name.to_string()),
            Target::File(path) => (EntryKind::File, path.to_string()),
        };
        Self {
            id: entry.id.0,
            kind,
            target,
            secret: entry.is_secret(),
            template: entry.template,
            executable: entry.executable,
            apply_to_new: entry.apply_to_new,
            value,
            created_at: entry.created_at,
            references: references.into_iter().collect(),
        }
    }
}

/// An entry as a member sees it: their own, or one the admin shares, whose secret value shows
/// nothing but its size.
#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "SetupEntry")]
pub(crate) struct EntryDto {
    #[serde(flatten)]
    #[ts(flatten)]
    entry: EntryFields,
    /// How the admin shares it, for an entry that is not the member's own.
    shared: Option<Shared>,
    /// The sandboxes it reaches, by name: those attached to it, or every one for a required
    /// entry.
    sandboxes: Vec<EntryUseDto>,
    /// Whether an entry made earlier has the same target, as only entries made before targets
    /// were unique can.
    duplicate: bool,
    /// For the member's own entry: whether it takes the place of an offered one.
    shadows: bool,
    /// For an offered entry: whether an entry of the member's takes its place.
    shadowed: bool,
}

/// A shared entry as the admin sees it.
#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "SharedEntry")]
pub(crate) struct SharedEntryDto {
    #[serde(flatten)]
    #[ts(flatten)]
    entry: EntryFields,
    shared: Shared,
    /// How many sandboxes are attached to it, none for a required entry, which reaches every one.
    attached: usize,
    /// The members with an entry of their own in its place, by handle.
    shadowed_by: Vec<Handle>,
}

/// An entry's attachment to one sandbox.
#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "EntryUse")]
pub(crate) struct EntryUseDto {
    sandbox: SandboxName,
    /// Whether the sandbox has a value of its own in place of the entry's.
    overridden: bool,
    /// For a template: the variables it refers to there that are not attached to the sandbox,
    /// which keep the sandbox from starting.
    unresolved: Vec<VariableName>,
}

impl From<Use> for EntryUseDto {
    fn from(attached: Use) -> Self {
        Self {
            sandbox: attached.sandbox,
            overridden: attached.overridden,
            unresolved: attached.unresolved.into_iter().collect(),
        }
    }
}

/// A template as one sandbox would get it at its next start, each secret variable masked.
#[derive(Serialize, TS)]
#[ts(rename = "TemplatePreview")]
pub(crate) struct PreviewDto {
    text: String,
    /// The variables it refers to that are not attached to the sandbox, left as written.
    unresolved: Vec<VariableName>,
}

impl From<Preview> for PreviewDto {
    fn from(preview: Preview) -> Self {
        Self {
            text: preview.text,
            unresolved: preview.unresolved.into_iter().collect(),
        }
    }
}

impl From<EntryView> for EntryDto {
    fn from(view: EntryView) -> Self {
        let EntryView {
            entry,
            sandboxes,
            references,
            duplicate,
            shadows,
            shadowed,
        } = view;
        let value = match entry.owner {
            Owner::Everyone(_) if entry.is_secret() => StoredValueDto::hidden(&entry.value),
            _ => StoredValueDto::from(&entry.value),
        };
        Self {
            entry: EntryFields::new(&entry, value, references),
            shared: entry.owner.shared(),
            sandboxes: sandboxes.into_iter().map(EntryUseDto::from).collect(),
            duplicate,
            shadows,
            shadowed,
        }
    }
}

impl From<SharedEntryView> for SharedEntryDto {
    fn from(view: SharedEntryView) -> Self {
        let SharedEntryView {
            entry,
            references,
            attached,
            shadowed_by,
        } = view;
        Self {
            shared: entry
                .owner
                .shared()
                .expect("the shared setup holds only shared entries"),
            entry: EntryFields::new(&entry, StoredValueDto::from(&entry.value), references),
            attached,
            shadowed_by,
        }
    }
}

/// An entry attached to a sandbox, with the value it has there in place of its own, if any.
#[derive(Serialize, TS)]
#[ts(rename = "AttachedEntry")]
pub(crate) struct AttachedDto {
    entry: EntryDto,
    #[serde(rename = "override")]
    overriding: Option<StoredValueDto>,
}

impl From<Attached> for AttachedDto {
    fn from(attached: Attached) -> Self {
        Self {
            overriding: attached.overriding.as_ref().map(StoredValueDto::from),
            entry: EntryDto::from(attached.view),
        }
    }
}
