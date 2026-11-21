use std::sync::Arc;

use chrono::Utc;
use secrecy::{ExposeSecret, SecretString};
use uuid::Uuid;

use super::{AccessPolicy, require_admin};
use crate::domain::{Handle, LocalAccount, Principal, Role, User};
use crate::ports::{LocalAccountRepo, PasswordHasher, UserRepo};
use crate::{Error, Result};

const MIN_PASSWORD_CHARS: usize = 12;

#[derive(Clone)]
pub struct LocalAccounts {
    accounts: Arc<dyn LocalAccountRepo>,
    users: Arc<dyn UserRepo>,
    hasher: Arc<dyn PasswordHasher>,
    policy: AccessPolicy,
    dummy_hash: String,
}

impl LocalAccounts {
    pub fn new(
        accounts: Arc<dyn LocalAccountRepo>,
        users: Arc<dyn UserRepo>,
        hasher: Arc<dyn PasswordHasher>,
        policy: AccessPolicy,
    ) -> Result<Self> {
        let dummy_hash = hasher.hash(&SecretString::from(Uuid::new_v4().to_string()))?;
        Ok(Self {
            accounts,
            users,
            hasher,
            policy,
            dummy_hash,
        })
    }

    pub async fn create(
        &self,
        actor: Option<&User>,
        username: &str,
        password: SecretString,
        role: Role,
    ) -> Result<()> {
        if let Some(actor) = actor {
            require_admin(actor)?;
        }
        let account = LocalAccount {
            username: Handle::parse(username)?,
            password_hash: self.hash(&password)?,
            role,
            created_at: Utc::now(),
        };
        self.accounts.insert(&account).await
    }

    pub async fn set_password(
        &self,
        actor: &User,
        username: &str,
        password: SecretString,
    ) -> Result<()> {
        require_admin(actor)?;
        let account = self.find(username).await?;
        let password_hash = self.hash(&password)?;
        self.accounts
            .update(&LocalAccount {
                password_hash,
                ..account
            })
            .await
    }

    pub async fn delete(&self, actor: &User, username: &str) -> Result<()> {
        require_admin(actor)?;
        let account = self.find(username).await?;
        let linked = self.users.by_subject(&subject(&account.username)).await?;
        if let Some(user) = linked.filter(|user| user.enabled) {
            self.users
                .update(&User {
                    enabled: false,
                    ..user
                })
                .await?;
        }
        self.accounts.delete(&account.username).await?;
        Ok(())
    }

    pub async fn list(&self, actor: &User) -> Result<Vec<LocalAccount>> {
        require_admin(actor)?;
        self.accounts.list().await
    }

    pub async fn authenticate(&self, username: &str, password: SecretString) -> Result<Principal> {
        let account = match Handle::parse(username) {
            Ok(username) => self.accounts.get(&username).await?,
            Err(_) => None,
        };
        let hash = account
            .as_ref()
            .map_or(&self.dummy_hash, |account| &account.password_hash);
        let verified = self.hasher.verify(&password, hash);
        match account {
            Some(account) if verified => Ok(Principal {
                subject: subject(&account.username),
                preferred_username: account.username.to_string(),
                email: None,
                groups: self.policy.groups(account.role),
            }),
            _ => Err(Error::Forbidden),
        }
    }

    fn hash(&self, password: &SecretString) -> Result<String> {
        if password.expose_secret().chars().count() < MIN_PASSWORD_CHARS {
            return Err(Error::Invalid(format!(
                "password must be at least {MIN_PASSWORD_CHARS} characters"
            )));
        }
        self.hasher.hash(password)
    }

    async fn find(&self, username: &str) -> Result<LocalAccount> {
        self.accounts
            .get(&Handle::parse(username)?)
            .await?
            .ok_or_else(|| Error::NotFound(format!("local account {username}")))
    }
}

fn subject(username: &Handle) -> String {
    format!("local:{username}")
}

#[cfg(test)]
mod tests {
    use secrecy::SecretString;

    use crate::Error;
    use crate::domain::{Principal, Role};
    use crate::testing::{ADAM_KEY, ADMINS, MEMBERS, World};

    const PASSWORD: &str = "correct horse battery";

    fn password(value: &str) -> SecretString {
        SecretString::from(value)
    }

    async fn world_with(accounts: &[(&str, Role)]) -> World {
        let w = World::new();
        for &(username, role) in accounts {
            w.local
                .create(None, username, password(PASSWORD), role)
                .await
                .unwrap();
        }
        w
    }

    #[tokio::test]
    async fn authenticate_maps_role_to_groups() {
        let w = world_with(&[("adam", Role::Member), ("root", Role::Admin)]).await;
        assert_eq!(
            w.local
                .authenticate("adam", password(PASSWORD))
                .await
                .unwrap(),
            Principal {
                subject: "local:adam".to_owned(),
                preferred_username: "adam".to_owned(),
                email: None,
                groups: vec![MEMBERS.to_owned()],
            }
        );
        let root = w
            .local
            .authenticate("root", password(PASSWORD))
            .await
            .unwrap();
        assert_eq!(root.subject, "local:root");
        assert_eq!(root.groups, [MEMBERS, ADMINS]);
        assert_eq!(w.accounts.login(root).await.unwrap().role, Role::Admin);
    }

    #[tokio::test]
    async fn wrong_password_and_unknown_user_look_identical() {
        let w = world_with(&[("adam", Role::Member)]).await;
        let attempts = [
            ("adam", "wrong horse battery"),
            ("nobody", PASSWORD),
            ("Not A Handle", PASSWORD),
        ];
        for (i, (username, attempt)) in attempts.into_iter().enumerate() {
            let before = w.hasher.verifications();
            let err = w
                .local
                .authenticate(username, password(attempt))
                .await
                .unwrap_err();
            assert!(matches!(err, Error::Forbidden), "attempt {i}: {err:?}");
            assert_eq!(w.hasher.verifications(), before + 1, "attempt {i}");
        }
    }

    #[tokio::test]
    async fn only_admin_or_cli_can_create() {
        let w = World::new();
        let eve = w.member("eve").await;
        let root = w.admin("root").await;
        assert!(matches!(
            w.local
                .create(Some(&eve), "adam", password(PASSWORD), Role::Admin)
                .await,
            Err(Error::Forbidden)
        ));
        w.local
            .create(None, "first", password(PASSWORD), Role::Admin)
            .await
            .unwrap();
        w.local
            .create(Some(&root), "adam", password(PASSWORD), Role::Member)
            .await
            .unwrap();
        assert!(matches!(
            w.local
                .create(
                    Some(&root),
                    "adam",
                    password("another password"),
                    Role::Admin
                )
                .await,
            Err(Error::Conflict(_))
        ));

        assert!(matches!(w.local.list(&eve).await, Err(Error::Forbidden)));
        assert!(matches!(
            w.local
                .set_password(&eve, "adam", password("taken over by eve"))
                .await,
            Err(Error::Forbidden)
        ));
        assert!(matches!(
            w.local.delete(&eve, "adam").await,
            Err(Error::Forbidden)
        ));
        let listed: Vec<(String, Role)> = w
            .local
            .list(&root)
            .await
            .unwrap()
            .into_iter()
            .map(|account| (account.username.to_string(), account.role))
            .collect();
        assert_eq!(
            listed,
            [
                ("adam".to_owned(), Role::Member),
                ("first".to_owned(), Role::Admin)
            ]
        );
        assert!(
            w.local
                .authenticate("adam", password(PASSWORD))
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn short_password_is_invalid() {
        let w = world_with(&[("adam", Role::Member)]).await;
        let root = w.admin("root").await;
        for short in ["elevenchars", "ééééééééééé"] {
            let err = w
                .local
                .create(None, "eve", password(short), Role::Member)
                .await
                .unwrap_err();
            assert!(matches!(err, Error::Invalid(_)), "{err:?}");
            assert!(!err.to_string().contains(short));
            assert!(matches!(
                w.local.set_password(&root, "adam", password(short)).await,
                Err(Error::Invalid(_))
            ));
        }
        assert!(matches!(
            w.local.authenticate("eve", password("elevenchars")).await,
            Err(Error::Forbidden)
        ));
        assert!(
            w.local
                .authenticate("adam", password(PASSWORD))
                .await
                .is_ok()
        );
        assert!(matches!(
            w.local
                .create(None, "Eve", password(PASSWORD), Role::Member)
                .await,
            Err(Error::Invalid(_))
        ));
        w.local
            .create(None, "eve", password("twelve chars"), Role::Member)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn admins_reset_passwords() {
        let w = world_with(&[("adam", Role::Member)]).await;
        let root = w.admin("root").await;
        w.local
            .set_password(&root, "adam", password("a brand new password"))
            .await
            .unwrap();
        assert!(matches!(
            w.local.authenticate("adam", password(PASSWORD)).await,
            Err(Error::Forbidden)
        ));
        let principal = w
            .local
            .authenticate("adam", password("a brand new password"))
            .await
            .unwrap();
        assert_eq!(principal.groups, [MEMBERS]);
        assert!(matches!(
            w.local
                .set_password(&root, "ghost", password(PASSWORD))
                .await,
            Err(Error::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn deleting_an_account_disables_its_user() {
        let w = world_with(&[("adam", Role::Member), ("never", Role::Member)]).await;
        let root = w.admin("root").await;
        let principal = w
            .local
            .authenticate("adam", password(PASSWORD))
            .await
            .unwrap();
        let adam = w.accounts.login(principal).await.unwrap();
        w.keys.add(&adam, "laptop", ADAM_KEY).await.unwrap();
        w.sandbox(&adam, "main").await;
        assert!(w.access.authenticate("adam", ADAM_KEY).await.unwrap());

        w.local.delete(&root, "adam").await.unwrap();
        assert!(!w.access.authenticate("adam", ADAM_KEY).await.unwrap());
        assert!(matches!(
            w.accounts.require_active(adam.id).await,
            Err(Error::Forbidden)
        ));
        assert!(matches!(
            w.local.authenticate("adam", password(PASSWORD)).await,
            Err(Error::Forbidden)
        ));
        assert!(matches!(
            w.local.delete(&root, "adam").await,
            Err(Error::NotFound(_))
        ));

        w.local.delete(&root, "never").await.unwrap();
        assert!(w.accounts.require_active(root.id).await.is_ok());
        assert_eq!(w.local.list(&root).await.unwrap(), vec![]);
    }
}
