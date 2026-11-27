use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use super::{Handle, PictureUrl};
use crate::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct UserId(pub Uuid);

impl UserId {
    /// Whom the secret store keeps the values of shared entries under, which no user is.
    pub const SHARED: UserId = UserId(Uuid::nil());
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Member,
    Admin,
}

/// Logins, sessions and SSH need `Active`. A `Deleting` user is being deleted, which only that
/// deletion may finish.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub enum UserStatus {
    Active,
    Disabled,
    Deleting,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct User {
    pub id: UserId,
    pub subject: String,
    pub handle: Handle,
    pub email: Option<String>,
    /// The identity provider's picture of the person, as of their last login.
    pub picture: Option<PictureUrl>,
    pub role: Role,
    pub status: UserStatus,
    pub created_at: DateTime<Utc>,
}

impl User {
    /// The subject a provider's identity gives the user who logs in with it.
    #[must_use]
    pub fn subject_of(provider: &str, subject: &str) -> String {
        format!("{provider}:{subject}")
    }

    /// The subject the user logs in with at `provider`, when they log in there.
    #[must_use]
    pub fn subject_at(&self, provider: &str) -> Option<&str> {
        self.subject.strip_prefix(provider)?.strip_prefix(':')
    }

    /// The provider the user logs in with: their subject before its first `:`.
    #[must_use]
    pub fn provider(&self) -> &str {
        self.subject
            .split_once(':')
            .map_or(self.subject.as_str(), |(provider, _)| provider)
    }

    /// Who the provider says the user is: their subject after its first `:`.
    #[must_use]
    pub fn subject_in_provider(&self) -> &str {
        self.subject
            .split_once(':')
            .map_or(self.subject.as_str(), |(_, subject)| subject)
    }
}

/// Proof that the acting user is an admin, for ports that act on an admin's behalf.
#[derive(Debug)]
pub struct Admin(Handle);

impl Admin {
    pub fn check(actor: &User) -> Result<Self> {
        if actor.role == Role::Admin {
            Ok(Self(actor.handle.clone()))
        } else {
            Err(Error::Forbidden)
        }
    }

    #[must_use]
    pub fn handle(&self) -> &Handle {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::{Admin, Role, User, UserStatus};
    use crate::Error;
    use crate::fakes::World;

    #[tokio::test]
    async fn only_admins_get_the_admin_proof() {
        let w = World::new();
        assert!(Admin::check(&w.admin("root").await).is_ok());
        assert!(matches!(
            Admin::check(&w.member("adam").await),
            Err(Error::Forbidden)
        ));
    }

    #[tokio::test]
    async fn a_user_logs_in_at_the_provider_of_their_subject_only() {
        let user = User {
            subject: User::subject_of("local", "adam"),
            ..World::new().member("adam").await
        };
        assert_eq!(user.subject_at("local"), Some("adam"));
        assert_eq!(user.provider(), "local");
        assert_eq!(user.subject_at("loc"), None);
        assert_eq!(user.subject_at("oidc"), None);
    }

    #[tokio::test]
    async fn the_subject_in_the_provider_follows_the_first_colon() {
        let member = World::new().member("adam").await;
        for (subject, inside) in [
            ("local:adam", "adam"),
            ("oidc:https://id.example/u:1", "https://id.example/u:1"),
            ("adam", "adam"),
        ] {
            let user = User {
                subject: subject.to_owned(),
                ..member.clone()
            };
            assert_eq!(user.subject_in_provider(), inside, "{subject}");
        }
    }

    #[test]
    fn role_serializes_lowercase() {
        for (role, text) in [(Role::Member, r#""member""#), (Role::Admin, r#""admin""#)] {
            assert_eq!(serde_json::to_string(&role).unwrap(), text);
            assert_eq!(serde_json::from_str::<Role>(text).unwrap(), role);
        }
        assert!(serde_json::from_str::<Role>(r#""root""#).is_err());
    }

    #[test]
    fn status_serializes_lowercase() {
        for (status, text) in [
            (UserStatus::Active, r#""active""#),
            (UserStatus::Disabled, r#""disabled""#),
            (UserStatus::Deleting, r#""deleting""#),
        ] {
            assert_eq!(serde_json::to_string(&status).unwrap(), text);
            assert_eq!(serde_json::from_str::<UserStatus>(text).unwrap(), status);
        }
    }
}
