use chrono::{DateTime, Utc};
use uuid::Uuid;

/// Where the secret store keeps one value. Each value written gets a new one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SecretId(pub Uuid);

impl SecretId {
    #[must_use]
    pub fn generate() -> Self {
        Self(Uuid::new_v4())
    }
}

/// A value of an entry, or of an entry in one sandbox, as Corral's database keeps it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredValue {
    pub kept: Kept,
    pub set_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kept {
    /// The value itself, shown in full.
    Plain(Vec<u8>),
    /// Kept in the secret store, never shown again.
    Secret(SecretRef),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SecretRef {
    pub id: SecretId,
    /// In bytes.
    pub size: usize,
    /// What a secret variable shows of itself; a secret file shows only its size.
    pub glimpse: Option<Glimpse>,
}

/// What may be shown of a secret text: its length in characters, with its first 4 and last 2
/// characters once it has at least 16, which leaves too little of it to guess.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Glimpse {
    pub length: usize,
    pub ends: Option<(String, String)>,
}

const GLIMPSE_MIN_LENGTH: usize = 16;
const HEAD: usize = 4;
const TAIL: usize = 2;

impl Glimpse {
    #[must_use]
    pub fn of(text: &str) -> Self {
        let chars: Vec<char> = text.chars().collect();
        let length = chars.len();
        let ends = (length >= GLIMPSE_MIN_LENGTH).then(|| {
            (
                chars[..HEAD].iter().collect(),
                chars[length - TAIL..].iter().collect(),
            )
        });
        Self { length, ends }
    }
}

impl StoredValue {
    #[must_use]
    pub fn is_secret(&self) -> bool {
        matches!(self.kept, Kept::Secret(_))
    }

    /// In bytes.
    #[must_use]
    pub fn size(&self) -> usize {
        match &self.kept {
            Kept::Plain(content) => content.len(),
            Kept::Secret(secret) => secret.size,
        }
    }

    #[must_use]
    pub fn secret_id(&self) -> Option<SecretId> {
        match &self.kept {
            Kept::Plain(_) => None,
            Kept::Secret(secret) => Some(secret.id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Glimpse;

    #[test]
    fn a_glimpse_shows_the_ends_only_of_16_characters_or_more() {
        assert_eq!(
            Glimpse::of("ghp_0123456789abcdefghijklmnopqrstuvwXz"),
            Glimpse {
                length: 39,
                ends: Some(("ghp_".to_owned(), "Xz".to_owned()))
            }
        );
        assert_eq!(
            Glimpse::of("0123456789abcdef").ends,
            Some(("0123".to_owned(), "ef".to_owned()))
        );
        for short in ["0123456789abcde", "hunter2", ""] {
            assert_eq!(
                Glimpse::of(short),
                Glimpse {
                    length: short.len(),
                    ends: None
                }
            );
        }
        assert_eq!(
            Glimpse::of("ééééxxxxxxxxxxxxxxxxàà"),
            Glimpse {
                length: 22,
                ends: Some(("éééé".to_owned(), "àà".to_owned()))
            }
        );
    }
}
