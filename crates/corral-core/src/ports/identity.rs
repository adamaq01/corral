use std::collections::BTreeMap;
use std::fmt;

use async_trait::async_trait;
use secrecy::SecretString;
use serde::Serialize;

use crate::Result;
use chrono::{DateTime, Utc};

use crate::domain::{Handle, Role};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    /// The provider's own stable id for the person.
    pub subject: String,
    pub username: String,
    pub email: Option<String>,
    pub role: Option<Role>,
}

#[must_use]
pub fn user_subject(provider: &str, subject: &str) -> String {
    format!("{provider}:{subject}")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Method {
    Password,
    Redirect,
}

#[derive(Clone, PartialEq, Eq)]
pub struct PendingLogin(pub String);

impl fmt::Debug for PendingLogin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PendingLogin(..)")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Challenge {
    Password,
    Redirect { url: String, pending: PendingLogin },
}

#[derive(Debug)]
pub enum Answer {
    Password {
        username: String,
        password: SecretString,
    },
    Redirect {
        pending: PendingLogin,
        params: BTreeMap<String, String>,
    },
}

#[async_trait]
pub trait IdentityProvider: Send + Sync {
    fn id(&self) -> &str;
    fn method(&self) -> Method;
    async fn challenge(&self) -> Result<Challenge>;
    /// `Forbidden` for a wrong answer. A verified person outside every role comes back with
    /// `role: None`, so their user can be disabled.
    async fn verify(&self, answer: Answer) -> Result<Identity>;
    /// The provider's own accounts, for a provider that keeps them.
    fn accounts(&self) -> Option<&dyn ManagedAccounts> {
        None
    }
}

/// An account a provider keeps, known by the subject it authenticates as.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManagedAccount {
    pub subject: String,
    pub username: Handle,
    pub role: Role,
    pub created_at: DateTime<Utc>,
}

#[async_trait]
pub trait ManagedAccounts: Send + Sync {
    /// `Conflict` when the username is taken.
    async fn create(&self, username: &Handle, password: SecretString, role: Role) -> Result<()>;
    /// `NotFound` when no account has the subject.
    async fn set_password(&self, subject: &str, password: SecretString) -> Result<()>;
    /// Whether the account existed.
    async fn delete(&self, subject: &str) -> Result<bool>;
    async fn list(&self) -> Result<Vec<ManagedAccount>>;
}

#[cfg(test)]
mod tests {
    use secrecy::SecretString;

    use super::{Answer, Challenge, PendingLogin};

    #[test]
    fn debug_never_prints_pending_logins_or_passwords() {
        let pending = PendingLogin("pkce-verifier-and-nonce".to_owned());
        let challenge = Challenge::Redirect {
            url: "https://idp.example.com/authorize?state=public".to_owned(),
            pending: pending.clone(),
        };
        let answers = [
            Answer::Redirect {
                pending,
                params: [("code".to_owned(), "c".to_owned())].into(),
            },
            Answer::Password {
                username: "adam".to_owned(),
                password: SecretString::from("correct horse battery"),
            },
        ];
        let printed = format!("{challenge:?} {challenge:#?} {answers:?}");
        assert!(!printed.contains("pkce-verifier-and-nonce"), "{printed}");
        assert!(!printed.contains("correct horse"), "{printed}");
        assert!(printed.contains("PendingLogin(..)"), "{printed}");
    }
}
