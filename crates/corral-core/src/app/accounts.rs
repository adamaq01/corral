use std::sync::Arc;

use chrono::Utc;
use secrecy::SecretString;
use uuid::Uuid;

use crate::domain::{Admin, Handle, Role, User, UserId};
use crate::ports::{
    Identity, IdentityProvider, ManagedAccount, ManagedAccounts, UserRepo, user_subject,
};
use crate::{Error, Result};

#[derive(Clone)]
pub struct Accounts {
    users: Arc<dyn UserRepo>,
    providers: Vec<Arc<dyn IdentityProvider>>,
}

impl Accounts {
    #[must_use]
    pub fn new(users: Arc<dyn UserRepo>, providers: Vec<Arc<dyn IdentityProvider>>) -> Self {
        Self { users, providers }
    }

    pub async fn login(&self, provider: &str, identity: Identity) -> Result<User> {
        let subject = user_subject(provider, &identity.subject);
        let existing = self.users.by_subject(&subject).await?;
        let Some(role) = identity.role else {
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
                    email: identity.email,
                    role,
                    ..user
                };
                self.users.update(&user).await?;
                Ok(user)
            }
            None => self.register(subject, identity, role).await,
        }
    }

    pub async fn require_active(&self, id: UserId) -> Result<User> {
        self.users
            .get(id)
            .await?
            .filter(|user| user.enabled)
            .ok_or(Error::Forbidden)
    }

    pub async fn set_enabled(&self, _: &Admin, target: UserId, enabled: bool) -> Result<User> {
        let user = self
            .users
            .get(target)
            .await?
            .ok_or_else(|| Error::NotFound(format!("user {}", target.0)))?;
        let user = User { enabled, ..user };
        self.users.update(&user).await?;
        Ok(user)
    }

    pub async fn list(&self, _: &Admin) -> Result<Vec<User>> {
        self.users.list().await
    }

    pub async fn managed_accounts(&self, _: &Admin, provider: &str) -> Result<Vec<ManagedAccount>> {
        self.managed(provider)?.list().await
    }

    pub async fn create_account(
        &self,
        _: &Admin,
        provider: &str,
        username: &Handle,
        password: SecretString,
        role: Role,
    ) -> Result<()> {
        self.managed(provider)?
            .create(username, password, role)
            .await
    }

    pub async fn set_password(
        &self,
        _: &Admin,
        provider: &str,
        subject: &str,
        password: SecretString,
    ) -> Result<()> {
        self.managed(provider)?
            .set_password(subject, password)
            .await
    }

    /// Removes the account from its provider and disables the user it logged in as.
    pub async fn delete_account(&self, _: &Admin, provider: &str, subject: &str) -> Result<()> {
        if !self.managed(provider)?.delete(subject).await? {
            return Err(Error::NotFound(format!("account {subject}")));
        }
        let linked = self
            .users
            .by_subject(&user_subject(provider, subject))
            .await?;
        if let Some(user) = linked.filter(|user| user.enabled) {
            self.users
                .update(&User {
                    enabled: false,
                    ..user
                })
                .await?;
        }
        Ok(())
    }

    fn managed(&self, provider: &str) -> Result<&dyn ManagedAccounts> {
        self.providers
            .iter()
            .find(|candidate| candidate.id() == provider)
            .and_then(|provider| provider.accounts())
            .ok_or_else(|| Error::NotFound(format!("accounts of provider {provider}")))
    }

    async fn register(&self, subject: String, identity: Identity, role: Role) -> Result<User> {
        let base = Handle::from_preferred_username(&identity.username);
        let mut handle = base.clone();
        let mut suffix = 1;
        while self.users.by_handle(&handle).await?.is_some() {
            suffix += 1;
            handle = base.with_suffix(suffix);
        }
        let user = User {
            id: UserId(Uuid::new_v4()),
            subject,
            handle,
            email: identity.email,
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
    use secrecy::SecretString;
    use uuid::Uuid;

    use crate::Error;
    use crate::domain::{Handle, Role, UserId};
    use crate::fakes::{PROVIDER, World, admin_proof, identity};
    use crate::ports::UserRepo;

    fn password(value: &str) -> SecretString {
        SecretString::from(value)
    }

    #[tokio::test]
    async fn login_requires_a_role() {
        let w = World::new();
        assert!(matches!(
            w.accounts
                .login(PROVIDER, identity("s1", "adam", None))
                .await,
            Err(Error::Forbidden)
        ));
        assert_eq!(w.users.by_subject("test:s1").await.unwrap(), None);
        let admin = w
            .accounts
            .login(PROVIDER, identity("s2", "root", Some(Role::Admin)))
            .await
            .unwrap();
        assert_eq!(admin.role, Role::Admin);
    }

    #[tokio::test]
    async fn login_assigns_unique_stable_handles() {
        let w = World::new();
        let first = w
            .accounts
            .login(PROVIDER, identity("s1", "adam", Some(Role::Member)))
            .await
            .unwrap();
        let second = w
            .accounts
            .login(PROVIDER, identity("s2", "Adam", Some(Role::Member)))
            .await
            .unwrap();
        let third = w
            .accounts
            .login(PROVIDER, identity("s3", "ADAM", Some(Role::Member)))
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
            .login(PROVIDER, identity("s1", "renamed", Some(Role::Admin)))
            .await
            .unwrap();
        assert_eq!((again.handle.as_str(), again.role), ("adam", Role::Admin));
        assert_eq!(again.id, first.id);
        assert_eq!(again.email.as_deref(), Some("renamed@example.com"));
        assert_eq!(w.users.get(first.id).await.unwrap(), Some(again));
    }

    #[tokio::test]
    async fn the_same_subject_from_two_providers_is_two_users() {
        let w = World::new();
        let member = || identity("s1", "adam", Some(Role::Member));
        let local = w.accounts.login("local", member()).await.unwrap();
        let oidc = w.accounts.login("oidc", member()).await.unwrap();
        assert_ne!(local.id, oidc.id);
        assert_eq!(
            (local.subject.as_str(), oidc.subject.as_str()),
            ("local:s1", "oidc:s1")
        );
    }

    #[tokio::test]
    async fn disabled_user_cannot_login() {
        let w = World::new();
        let root = w.admin("root").await;
        let eve = w.member("eve").await;
        w.accounts
            .set_enabled(&admin_proof(&root), eve.id, false)
            .await
            .unwrap();
        assert!(matches!(
            w.accounts
                .login(PROVIDER, identity("eve", "eve", Some(Role::Member)))
                .await,
            Err(Error::Forbidden)
        ));
        assert!(matches!(
            w.accounts.require_active(eve.id).await,
            Err(Error::Forbidden)
        ));

        w.accounts
            .set_enabled(&admin_proof(&root), eve.id, true)
            .await
            .unwrap();
        let back = w
            .accounts
            .login(PROVIDER, identity("eve", "eve", Some(Role::Member)))
            .await
            .unwrap();
        assert_eq!(back.id, eve.id);
        assert_eq!(w.accounts.require_active(eve.id).await.unwrap(), back);
    }

    #[tokio::test]
    async fn losing_every_role_disables_the_user() {
        let w = World::new();
        let adam = w
            .accounts
            .login(PROVIDER, identity("s1", "adam", Some(Role::Member)))
            .await
            .unwrap();
        assert!(matches!(
            w.accounts
                .login(PROVIDER, identity("s1", "adam", None))
                .await,
            Err(Error::Forbidden)
        ));
        assert!(
            !w.users
                .by_subject("test:s1")
                .await
                .unwrap()
                .unwrap()
                .enabled
        );
        assert!(matches!(
            w.accounts
                .login(PROVIDER, identity("s1", "adam", Some(Role::Member)))
                .await,
            Err(Error::Forbidden)
        ));

        let root = w.admin("root").await;
        w.accounts
            .set_enabled(&admin_proof(&root), adam.id, true)
            .await
            .unwrap();
        assert!(
            w.accounts
                .login(PROVIDER, identity("s1", "adam", Some(Role::Member)))
                .await
                .unwrap()
                .enabled
        );
    }

    #[tokio::test]
    async fn admins_list_users_and_enable_only_existing_ones() {
        let w = World::new();
        let adam = w.member("adam").await;
        let root = w.admin("root").await;
        let by = admin_proof(&root);
        assert_eq!(w.accounts.require_active(root.id).await.unwrap(), root);
        assert_eq!(
            w.accounts.list(&by).await.unwrap(),
            vec![adam, root.clone()]
        );
        assert!(matches!(
            w.accounts
                .set_enabled(&by, UserId(Uuid::new_v4()), false)
                .await,
            Err(Error::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn admins_manage_the_accounts_a_provider_keeps() {
        let w = World::new();
        let by = admin_proof(&w.admin("root").await);
        let eve = Handle::parse("eve").unwrap();
        w.accounts
            .create_account(&by, PROVIDER, &eve, password("first"), Role::Member)
            .await
            .unwrap();
        assert!(matches!(
            w.accounts
                .create_account(&by, PROVIDER, &eve, password("again"), Role::Admin)
                .await,
            Err(Error::Conflict(_))
        ));
        w.accounts
            .set_password(&by, PROVIDER, "eve", password("second"))
            .await
            .unwrap();
        assert!(matches!(
            w.accounts
                .set_password(&by, PROVIDER, "ghost", password("x"))
                .await,
            Err(Error::NotFound(_))
        ));
        let listed: Vec<(String, Role)> = w
            .accounts
            .managed_accounts(&by, PROVIDER)
            .await
            .unwrap()
            .into_iter()
            .map(|account| (account.subject, account.role))
            .collect();
        assert_eq!(listed, [("eve".to_owned(), Role::Member)]);

        for absent in ["oidc", "nope"] {
            assert!(matches!(
                w.accounts.managed_accounts(&by, absent).await,
                Err(Error::NotFound(_))
            ));
        }
    }

    #[tokio::test]
    async fn deleting_an_account_disables_the_user_it_logged_in_as() {
        let w = World::new();
        let by = admin_proof(&w.admin("root").await);
        w.accounts
            .create_account(
                &by,
                PROVIDER,
                &Handle::parse("eve").unwrap(),
                password("p"),
                Role::Member,
            )
            .await
            .unwrap();
        let eve = w.member("eve").await;

        w.accounts
            .delete_account(&by, PROVIDER, "eve")
            .await
            .unwrap();
        assert!(matches!(
            w.accounts.require_active(eve.id).await,
            Err(Error::Forbidden)
        ));
        assert!(matches!(
            w.accounts.delete_account(&by, PROVIDER, "eve").await,
            Err(Error::NotFound(_))
        ));
        assert_eq!(
            w.accounts.managed_accounts(&by, PROVIDER).await.unwrap(),
            []
        );
    }
}
