use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::Handle;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct UserId(pub Uuid);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Member,
    Admin,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct User {
    pub id: UserId,
    pub subject: String,
    pub handle: Handle,
    pub email: Option<String>,
    pub role: Role,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::Role;

    #[test]
    fn role_serializes_lowercase() {
        for (role, text) in [(Role::Member, r#""member""#), (Role::Admin, r#""admin""#)] {
            assert_eq!(serde_json::to_string(&role).unwrap(), text);
            assert_eq!(serde_json::from_str::<Role>(text).unwrap(), role);
        }
        assert!(serde_json::from_str::<Role>(r#""root""#).is_err());
    }
}
