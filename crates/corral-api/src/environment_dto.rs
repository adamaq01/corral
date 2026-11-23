use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use chrono::{DateTime, Utc};
use corral_core::app::{Attached, EntryChange, EntryView, NewEntry};
use corral_core::domain::{
    CredentialName, Entry, HomePath, Kept, SandboxName, StoredValue, Target,
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
pub(crate) struct NewEntryDto {
    kind: EntryKind,
    /// The variable's name, or the file's path relative to the home.
    target: String,
    secret: bool,
    #[serde(default)]
    template: bool,
    apply_to_new: bool,
    value: ValueInput,
}

impl NewEntryDto {
    pub(crate) fn parse(self) -> Result<NewEntry> {
        let target = match self.kind {
            EntryKind::Variable => Target::Variable(CredentialName::parse(&self.target)?),
            EntryKind::File => Target::File(HomePath::parse(&self.target)?),
        };
        Ok(NewEntry {
            target,
            secret: self.secret,
            template: self.template,
            apply_to_new: self.apply_to_new,
            value: self.value.bytes()?,
        })
    }
}

/// An entry's flags, and its value when given; without one, the value stays.
#[derive(Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "EntryChange")]
pub(crate) struct EntryChangeDto {
    template: bool,
    apply_to_new: bool,
    #[ts(optional)]
    value: Option<ValueInput>,
}

impl EntryChangeDto {
    pub(crate) fn parse(self) -> Result<EntryChange> {
        Ok(EntryChange {
            value: self.value.map(ValueInput::bytes).transpose()?,
            template: self.template,
            apply_to_new: self.apply_to_new,
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
    /// A secret file, of which only the size shows.
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

#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "EnvironmentEntry")]
pub(crate) struct EntryDto {
    id: Uuid,
    kind: EntryKind,
    /// The variable's name, or the file's path relative to the home.
    target: String,
    secret: bool,
    /// For a text file: each `{{ NAME }}` takes the value of the variable `NAME` attached to
    /// the same sandbox.
    template: bool,
    /// Whether each new sandbox is attached to it.
    apply_to_new: bool,
    value: StoredValueDto,
    created_at: DateTime<Utc>,
    /// The sandboxes it is attached to.
    sandboxes: Vec<SandboxName>,
}

impl From<EntryView> for EntryDto {
    fn from(view: EntryView) -> Self {
        let EntryView { entry, sandboxes } = view;
        let Entry {
            id,
            target,
            template,
            apply_to_new,
            value,
            created_at,
            ..
        } = &entry;
        let (kind, target) = match target {
            Target::Variable(name) => (EntryKind::Variable, name.to_string()),
            Target::File(path) => (EntryKind::File, path.to_string()),
        };
        Self {
            id: id.0,
            kind,
            target,
            secret: entry.is_secret(),
            template: *template,
            apply_to_new: *apply_to_new,
            value: StoredValueDto::from(value),
            created_at: *created_at,
            sandboxes,
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
