use std::fmt;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::{Handle, SandboxName};
use crate::{Error, Result};

/// `<handle>`, or `<sandbox>.<handle>`. Neither part holds a dot, and nothing in either needs
/// encoding in an `ssh://` URI.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(try_from = "String", into = "String")]
#[ts(type = "string")]
pub struct SshUsername {
    pub handle: Handle,
    pub sandbox: Option<SandboxName>,
}

impl SshUsername {
    pub fn parse(value: &str) -> Result<Self> {
        let (sandbox, handle) = match value.split_once('.') {
            Some((sandbox, handle)) => (Some(SandboxName::parse(sandbox)?), handle),
            None => (None, value),
        };
        Ok(Self {
            handle: Handle::parse(handle)?,
            sandbox,
        })
    }
}

impl fmt::Display for SshUsername {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.sandbox {
            Some(sandbox) => write!(f, "{sandbox}.{}", self.handle),
            None => write!(f, "{}", self.handle),
        }
    }
}

impl TryFrom<String> for SshUsername {
    type Error = Error;

    fn try_from(value: String) -> Result<Self> {
        Self::parse(&value)
    }
}

impl From<SshUsername> for String {
    fn from(username: SshUsername) -> Self {
        username.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::SshUsername;

    #[test]
    fn ssh_username_with_sandbox() {
        let u = SshUsername::parse("main.adam").unwrap();
        assert_eq!(u.handle.as_str(), "adam");
        assert_eq!(u.sandbox.unwrap().as_str(), "main");
    }

    #[test]
    fn ssh_username_bare_handle_means_default() {
        assert!(SshUsername::parse("adam").unwrap().sandbox.is_none());
    }

    #[test]
    fn ssh_username_rejects_bad_input() {
        for bad in [
            "",
            ".adam",
            "main.",
            "main.Adam",
            "ma in.adam",
            "a.b.c",
            "adam+main",
            "main+adam",
            "adam main",
            &"a".repeat(33),
        ] {
            assert!(SshUsername::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn displays_and_serializes_as_the_login_string() {
        for login in ["adam", "main.adam"] {
            let u = SshUsername::parse(login).unwrap();
            assert_eq!(u.to_string(), login);
            assert_eq!(serde_json::to_string(&u).unwrap(), format!("\"{login}\""));
            assert_eq!(
                serde_json::from_str::<SshUsername>(&format!("\"{login}\"")).unwrap(),
                u
            );
        }
        assert!(serde_json::from_str::<SshUsername>(r#""a.b.c""#).is_err());
    }
}
