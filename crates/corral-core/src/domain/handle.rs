use std::fmt;

use serde::{Deserialize, Serialize};

use super::{SLUG_MAX_LEN, is_slug, is_slug_char};
use crate::{Error, Result};

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Handle(String);

impl Handle {
    pub fn parse(value: &str) -> Result<Self> {
        Self::try_from(value.to_owned())
    }

    #[must_use]
    pub fn from_preferred_username(preferred: &str) -> Self {
        let mapped: String = preferred
            .chars()
            .map(|c| {
                let c = c.to_ascii_lowercase();
                if is_slug_char(c) { c } else { '-' }
            })
            .collect();
        let trimmed = mapped.trim_matches('-');
        let truncated = trimmed[..trimmed.len().min(SLUG_MAX_LEN)].trim_end_matches('-');
        if truncated.is_empty() {
            Self("user".to_owned())
        } else {
            Self(truncated.to_owned())
        }
    }

    #[must_use]
    pub fn with_suffix(&self, n: u32) -> Self {
        let suffix = format!("-{n}");
        let base = &self.0[..self.0.len().min(SLUG_MAX_LEN - suffix.len())];
        Self(format!("{base}{suffix}"))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Handle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for Handle {
    type Error = Error;

    fn try_from(value: String) -> Result<Self> {
        if is_slug(&value) {
            Ok(Self(value))
        } else {
            Err(Error::Invalid(format!(
                "handle {value:?} must match [a-z0-9-]{{1,32}}"
            )))
        }
    }
}

impl From<Handle> for String {
    fn from(handle: Handle) -> Self {
        handle.0
    }
}

#[cfg(test)]
mod tests {
    use super::Handle;
    use crate::Error;

    #[test]
    fn parse_accepts_only_lowercase_slugs_up_to_32() {
        assert_eq!(Handle::parse("adam-2").unwrap().as_str(), "adam-2");
        assert!(Handle::parse(&"a".repeat(32)).is_ok());
        for bad in [
            "",
            "Adam",
            "ad am",
            "adam_x",
            "adam+main",
            "é",
            &"a".repeat(33),
        ] {
            assert!(
                matches!(Handle::parse(bad), Err(Error::Invalid(_))),
                "{bad}"
            );
        }
    }

    #[test]
    fn handle_from_preferred_username() {
        assert_eq!(
            Handle::from_preferred_username("Adam.Thibert").as_str(),
            "adam-thibert"
        );
        assert_eq!(Handle::from_preferred_username("!!!").as_str(), "user");
        assert_eq!(
            Handle::parse("adam").unwrap().with_suffix(2).as_str(),
            "adam-2"
        );
    }

    #[test]
    fn from_preferred_username_trims_and_truncates() {
        assert_eq!(Handle::from_preferred_username("-_Zoé_-").as_str(), "zo");
        let long = Handle::from_preferred_username(&format!("{}-b", "a".repeat(31)));
        assert_eq!(long.as_str(), "a".repeat(31));
    }

    #[test]
    fn with_suffix_truncates_the_base_to_fit() {
        let full = Handle::parse(&"a".repeat(32)).unwrap();
        assert_eq!(
            full.with_suffix(10).as_str(),
            format!("{}-10", "a".repeat(29))
        );
    }

    #[test]
    fn serde_uses_the_validated_string() {
        let handle: Handle = serde_json::from_str(r#""adam""#).unwrap();
        assert_eq!(serde_json::to_string(&handle).unwrap(), r#""adam""#);
        assert!(serde_json::from_str::<Handle>(r#""Adam""#).is_err());
    }
}
