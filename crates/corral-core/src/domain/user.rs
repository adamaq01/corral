use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::Handle;
use crate::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct UserId(pub Uuid);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    Member,
    Admin,
}

impl Role {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "member" => Ok(Self::Member),
            "admin" => Ok(Self::Admin),
            _ => Err(Error::Invalid(format!(
                "role {value:?} must be member or admin"
            ))),
        }
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Member => "member",
            Self::Admin => "admin",
        }
    }
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
    use crate::Error;

    #[test]
    fn role_round_trips_through_its_stored_string() {
        for role in [Role::Member, Role::Admin] {
            assert_eq!(Role::parse(role.as_str()).unwrap(), role);
        }
        assert!(matches!(Role::parse("root"), Err(Error::Invalid(_))));
    }
}
