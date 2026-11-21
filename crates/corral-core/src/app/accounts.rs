use std::sync::Arc;

use chrono::Utc;
use uuid::Uuid;

use super::require_admin;
use crate::domain::{Handle, Principal, Role, User, UserId};
use crate::ports::UserRepo;
use crate::{Error, Result};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessPolicy {
    pub member_group: String,
    pub admin_group: String,
}

impl AccessPolicy {
    fn role(&self, groups: &[String]) -> Option<Role> {
        if groups.contains(&self.admin_group) {
            Some(Role::Admin)
        } else if groups.contains(&self.member_group) {
            Some(Role::Member)
        } else {
            None
        }
    }

    pub(super) fn groups(&self, role: Role) -> Vec<String> {
        match role {
            Role::Member => vec![self.member_group.clone()],
            Role::Admin => vec![self.member_group.clone(), self.admin_group.clone()],
        }
    }
}

#[derive(Clone)]
pub struct Accounts {
    users: Arc<dyn UserRepo>,
    policy: AccessPolicy,
}

impl Accounts {
    #[must_use]
    pub fn new(users: Arc<dyn UserRepo>, policy: AccessPolicy) -> Self {
        Self { users, policy }
    }

    pub async fn login(&self, principal: Principal) -> Result<User> {
        let existing = self.users.by_subject(&principal.subject).await?;
        let Some(role) = self.policy.role(&principal.groups) else {
            if let Some(user) = existing.filter(|user| user.enabled) {
                self.users
                    .update(&User {
                        enabled: false,
                        ..user
                    })
                    .await?;
            }
            return Err(Error::Forbidden);
        };
        match existing {
            Some(user) if !user.enabled => Err(Error::Forbidden),
            Some(user) => {
                let user = User {
                    email: principal.email,
                    role,
                    ..user
                };
                self.users.update(&user).await?;
                Ok(user)
            }
            None => self.register(principal, role).await,
        }
    }

    pub async fn require_active(&self, id: UserId) -> Result<User> {
        self.users
            .get(id)
            .await?
            .filter(|user| user.enabled)
            .ok_or(Error::Forbidden)
    }

    pub async fn set_enabled(&self, actor: &User, target: UserId, enabled: bool) -> Result<User> {
        require_admin(actor)?;
        let user = self
            .users
            .get(target)
            .await?
            .ok_or_else(|| Error::NotFound(format!("user {}", target.0)))?;
        let user = User { enabled, ..user };
        self.users.update(&user).await?;
        Ok(user)
    }

    pub async fn list(&self, actor: &User) -> Result<Vec<User>> {
        require_admin(actor)?;
        self.users.list().await
    }

    async fn register(&self, principal: Principal, role: Role) -> Result<User> {
        let base = Handle::from_preferred_username(&principal.preferred_username);
        let mut handle = base.clone();
        let mut suffix = 1;
        while self.users.by_handle(&handle).await?.is_some() {
            suffix += 1;
            handle = base.with_suffix(suffix);
        }
        let user = User {
            id: UserId(Uuid::new_v4()),
            subject: principal.subject,
            handle,
            email: principal.email,
            role,
            enabled: true,
            created_at: Utc::now(),
        };
        self.users.insert(&user).await?;
        Ok(user)
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use crate::Error;
    use crate::domain::{Role, UserId};
    use crate::ports::UserRepo;
    use crate::testing::{ADMINS, MEMBERS, World, principal};

    #[tokio::test]
    async fn login_requires_member_group() {
        let w = World::new();
        assert!(matches!(
            w.accounts.login(principal("s1", "adam", &["other"])).await,
            Err(Error::Forbidden)
        ));
        assert_eq!(w.users.by_subject("s1").await.unwrap(), None);
        let admin = w
            .accounts
            .login(principal("s2", "root", &[ADMINS]))
            .await
            .unwrap();
        assert_eq!(admin.role, Role::Admin);
    }

    #[tokio::test]
    async fn login_assigns_unique_stable_handles() {
        let w = World::new();
        let first = w
            .accounts
            .login(principal("s1", "adam", &[MEMBERS]))
            .await
            .unwrap();
        let second = w
            .accounts
            .login(principal("s2", "Adam", &[MEMBERS]))
            .await
            .unwrap();
        let third = w
            .accounts
            .login(principal("s3", "ADAM", &[MEMBERS]))
            .await
            .unwrap();
        assert_eq!(
            (
                first.handle.as_str(),
                second.handle.as_str(),
                third.handle.as_str()
            ),
            ("adam", "adam-2", "adam-3")
        );
        assert_eq!(first.role, Role::Member);

        let again = w
            .accounts
            .login(principal("s1", "renamed", &[ADMINS]))
            .await
            .unwrap();
        assert_eq!((again.handle.as_str(), again.role), ("adam", Role::Admin));
        assert_eq!(again.id, first.id);
        assert_eq!(again.email.as_deref(), Some("renamed@example.com"));
        assert_eq!(w.users.get(first.id).await.unwrap(), Some(again));
    }

    #[tokio::test]
    async fn disabled_user_cannot_login() {
        let w = World::new();
        let root = w.admin("root").await;
        let eve = w.member("eve").await;
        w.accounts.set_enabled(&root, eve.id, false).await.unwrap();
        assert!(matches!(
            w.accounts.login(principal("eve", "eve", &[MEMBERS])).await,
            Err(Error::Forbidden)
        ));
        assert!(matches!(
            w.accounts.require_active(eve.id).await,
            Err(Error::Forbidden)
        ));

        w.accounts.set_enabled(&root, eve.id, true).await.unwrap();
        let back = w
            .accounts
            .login(principal("eve", "eve", &[MEMBERS]))
            .await
            .unwrap();
        assert_eq!(back.id, eve.id);
        assert_eq!(w.accounts.require_active(eve.id).await.unwrap(), back);
    }

    #[tokio::test]
    async fn leaving_the_group_disables_the_user() {
        let w = World::new();
        let adam = w
            .accounts
            .login(principal("s1", "adam", &[MEMBERS]))
            .await
            .unwrap();
        assert!(matches!(
            w.accounts.login(principal("s1", "adam", &[])).await,
            Err(Error::Forbidden)
        ));
        assert!(!w.users.by_subject("s1").await.unwrap().unwrap().enabled);
        assert!(matches!(
            w.accounts.login(principal("s1", "adam", &[MEMBERS])).await,
            Err(Error::Forbidden)
        ));

        let root = w.admin("root").await;
        w.accounts.set_enabled(&root, adam.id, true).await.unwrap();
        assert!(
            w.accounts
                .login(principal("s1", "adam", &[MEMBERS]))
                .await
                .unwrap()
                .enabled
        );
    }

    #[tokio::test]
    async fn only_admins_manage_users() {
        let w = World::new();
        let adam = w.member("adam").await;
        let root = w.admin("root").await;
        assert!(matches!(
            w.accounts.list(&adam).await,
            Err(Error::Forbidden)
        ));
        assert!(matches!(
            w.accounts.set_enabled(&adam, root.id, false).await,
            Err(Error::Forbidden)
        ));
        assert_eq!(w.accounts.require_active(root.id).await.unwrap(), root);
        assert_eq!(
            w.accounts.list(&root).await.unwrap(),
            vec![adam, root.clone()]
        );
        assert!(matches!(
            w.accounts
                .set_enabled(&root, UserId(Uuid::new_v4()), false)
                .await,
            Err(Error::NotFound(_))
        ));
    }
}
