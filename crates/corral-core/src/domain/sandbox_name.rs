use std::fmt;

use serde::{Deserialize, Serialize};

use super::is_slug;
use crate::{Error, Result};

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SandboxName(String);

impl SandboxName {
    pub fn parse(value: &str) -> Result<Self> {
        Self::try_from(value.to_owned())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SandboxName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for SandboxName {
    type Error = Error;

    fn try_from(value: String) -> Result<Self> {
        if is_slug(&value) {
            Ok(Self(value))
        } else {
            Err(Error::Invalid(format!(
                "sandbox name {value:?} must match [a-z0-9-]{{1,32}}"
            )))
        }
    }
}

impl From<SandboxName> for String {
    fn from(name: SandboxName) -> Self {
        name.0
    }
}

#[cfg(test)]
mod tests {
    use super::SandboxName;
    use crate::Error;

    #[test]
    fn parse_accepts_only_lowercase_slugs_up_to_32() {
        assert_eq!(SandboxName::parse("main-2").unwrap().as_str(), "main-2");
        assert!(SandboxName::parse(&"m".repeat(32)).is_ok());
        for bad in ["", "Main", "ma in", "main_x", "a+b", &"m".repeat(33)] {
            assert!(
                matches!(SandboxName::parse(bad), Err(Error::Invalid(_))),
                "{bad}"
            );
        }
    }

    #[test]
    fn serde_uses_the_validated_string() {
        let name: SandboxName = serde_json::from_str(r#""main""#).unwrap();
        assert_eq!(serde_json::to_string(&name).unwrap(), r#""main""#);
        assert!(serde_json::from_str::<SandboxName>(r#""ma in""#).is_err());
    }
}
