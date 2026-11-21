use std::fmt;

use chrono::{DateTime, Utc};

use super::{Handle, Role};

#[derive(Clone, PartialEq, Eq)]
pub struct LocalAccount {
    pub username: Handle,
    pub password_hash: String,
    pub role: Role,
    pub created_at: DateTime<Utc>,
}

impl fmt::Debug for LocalAccount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalAccount")
            .field("username", &self.username)
            .field("role", &self.role)
            .field("created_at", &self.created_at)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::LocalAccount;
    use crate::domain::{Handle, Role};

    #[test]
    fn debug_never_prints_the_password_hash() {
        let account = LocalAccount {
            username: Handle::parse("adam").unwrap(),
            password_hash: "$argon2id$v=19$m=19456,t=2,p=1$c2FsdA$aGFzaA".to_owned(),
            role: Role::Admin,
            created_at: Utc::now(),
        };
        let printed = format!("{account:?} {account:#?}");
        assert!(!printed.contains("argon2"), "{printed}");
        assert!(
            printed.contains("adam") && printed.contains("Admin"),
            "{printed}"
        );
    }
}
