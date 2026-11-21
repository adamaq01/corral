use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CredentialName(String);

impl CredentialName {
    pub fn parse(value: &str) -> Result<Self> {
        Self::try_from(value.to_owned())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn is_credential_name(value: &str) -> bool {
    let bytes = value.as_bytes();
    matches!(bytes.first(), Some(b'A'..=b'Z' | b'_'))
        && bytes.len() <= 64
        && bytes
            .iter()
            .all(|b| matches!(b, b'A'..=b'Z' | b'0'..=b'9' | b'_'))
}

impl fmt::Display for CredentialName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for CredentialName {
    type Error = Error;

    fn try_from(value: String) -> Result<Self> {
        if is_credential_name(&value) {
            Ok(Self(value))
        } else {
            Err(Error::Invalid(format!(
                "credential name {value:?} must match [A-Z_][A-Z0-9_]{{0,63}}"
            )))
        }
    }
}

impl From<CredentialName> for String {
    fn from(name: CredentialName) -> Self {
        name.0
    }
}

#[cfg(test)]
mod tests {
    use super::CredentialName;
    use crate::Error;

    #[test]
    fn credential_name_rules() {
        assert!(CredentialName::parse("ANTHROPIC_API_KEY").is_ok());
        assert!(CredentialName::parse("_X").is_ok());
        assert!(CredentialName::parse(&"K".repeat(64)).is_ok());
        for bad in ["anthropic", "1KEY", "KEY-1", "", &"K".repeat(65)] {
            assert!(
                matches!(CredentialName::parse(bad), Err(Error::Invalid(_))),
                "{bad}"
            );
        }
    }

    #[test]
    fn serde_uses_the_validated_string() {
        let name: CredentialName = serde_json::from_str(r#""GH_TOKEN""#).unwrap();
        assert_eq!(serde_json::to_string(&name).unwrap(), r#""GH_TOKEN""#);
        assert!(serde_json::from_str::<CredentialName>(r#""gh_token""#).is_err());
    }
}
