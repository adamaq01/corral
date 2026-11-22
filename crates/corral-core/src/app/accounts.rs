use std::sync::Arc;

use chrono::Utc;
use uuid::Uuid;

use super::{Credentials, Sandboxes};
use crate::domain::{Admin, Handle, Role, User, UserId, UserStatus};
use crate::ports::{Identity, IdentityProvider, KeyRepo, UserRepo};
use crate::{Error, Result};

#[derive(Clone)]
pub struct Accounts {
    users: Arc<dyn UserRepo>,
    keys: Arc<dyn KeyRepo>,
    sandboxes: Sandboxes,
    credentials: Credentials,
    providers: Vec<Arc<dyn IdentityProvider>>,
}

impl Accounts {
    #[must_use]
    pub fn new(
        users: Arc<dyn UserRepo>,
        keys: Arc<dyn KeyRepo>,
        sandboxes: Sandboxes,
        credentials: Credentials,
        providers: Vec<Arc<dyn IdentityProvider>>,
    ) -> Self {
        Self {
            users,
            keys,
            sandboxes,
            credentials,
            providers,
        }
    }

    pub async fn login(&self, provider: &str, identity: Identity) -> Result<User> {
        let subject = user_subject(provider, &identity.subject);
        let existing = self.users.by_subject(&subject).await?;
        let Some(role) = identity.role else {
            if let Some(user) = existing.filter(|user| user.status == UserStatus::Active) {
                self.users
                    .update(&User {
                        status: UserStatus::Disabled,
                        ..user
                    })
                    .await?;
            }
            return Err(Error::Forbidden);
        };
        match existing {
            Some(user) if user.status != UserStatus::Active => Err(Error::Forbidden),
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
            .filter(|user| user.status == UserStatus::Active)
            .ok_or(Error::Forbidden)
    }

    /// `Conflict` for a user being deleted: only finishing the deletion moves them on.
    pub async fn set_enabled(&self, _: &Admin, target: UserId, enabled: bool) -> Result<User> {
        let user = self.get(target).await?;
        if user.status == UserStatus::Deleting {
            return Err(Error::Conflict(format!(
                "user {} is being deleted",
                user.handle
            )));
        }
        let status = if enabled {
            UserStatus::Active
        } else {
            UserStatus::Disabled
        };
        let user = User { status, ..user };
        self.users.update(&user).await?;
        Ok(user)
    }

    pub async fn list(&self, _: &Admin) -> Result<Vec<User>> {
        self.users.list().await
    }

    /// Deletes the user the provider's subject logged in as, if any, else only has the provider
    /// forget the subject. Whether there was such a user.
    pub async fn delete_identity(&self, _: &Admin, provider: &str, subject: &str) -> Result<bool> {
        let subject_of_user = user_subject(provider, subject);
        if let Some(user) = self.users.by_subject(&subject_of_user).await? {
            self.erase(user).await?;
            return Ok(true);
        }
        if let Some(provider) = self.provider(provider) {
            provider.forget(subject).await?;
        }
        Ok(false)
    }

    pub async fn delete_user(&self, _: &Admin, id: UserId) -> Result<()> {
        self.erase(self.get(id).await?).await
    }

    /// A user deletes themselves, or finishes a deletion of theirs that failed part way, but
    /// never while disabled.
    pub async fn delete_self(&self, id: UserId) -> Result<()> {
        let user = self.users.get(id).await?.ok_or(Error::Forbidden)?;
        if user.status == UserStatus::Disabled {
            return Err(Error::Forbidden);
        }
        self.erase(user).await
    }

    /// Deletes the user and everything they own, each step safe to repeat, so running it again
    /// after a failure finishes the job. The user is marked deleting first, so logins, sessions
    /// and SSH stop at once, and their row goes last, so a retry still finds them.
    async fn erase(&self, user: User) -> Result<()> {
        if user.status != UserStatus::Deleting {
            self.users
                .update(&User {
                    status: UserStatus::Deleting,
                    ..user.clone()
                })
                .await?;
        }
        self.sandboxes.delete_all(user.id).await?;
        for key in self.keys.list(user.id).await? {
            self.keys.delete(user.id, key.id).await?;
        }
        self.credentials.remove_all(user.id).await?;
        if let Some((provider, subject)) = user.subject.split_once(':')
            && let Some(provider) = self.provider(provider)
        {
            provider.forget(subject).await?;
        }
        self.users.delete(user.id).await?;
        Ok(())
    }

    async fn get(&self, id: UserId) -> Result<User> {
        self.users
            .get(id)
            .await?
            .ok_or_else(|| Error::NotFound(format!("user {}", id.0)))
    }

    fn provider(&self, id: &str) -> Option<&dyn IdentityProvider> {
        self.providers
            .iter()
            .find(|provider| provider.id() == id)
            .map(AsRef::as_ref)
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
            status: UserStatus::Active,
            created_at: Utc::now(),
        };
        self.users.insert(&user).await?;
        Ok(user)
    }
}

fn user_subject(provider: &str, subject: &str) -> String {
    format!("{provider}:{subject}")
}

#[cfg(test)]
mod tests {
    use secrecy::SecretString;
    use uuid::Uuid;

    use crate::Error;
    use crate::domain::{Role, User, UserId, UserStatus};
    use crate::fakes::{ADAM_KEY, EVE_KEY, PROVIDER, World, admin_proof, credential, identity};
    use crate::ports::{SandboxEngine, UserRepo};

    fn secret(value: &str) -> SecretString {
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
        assert_eq!(
            w.users.by_subject("test:s1").await.unwrap().unwrap().status,
            UserStatus::Disabled
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
        assert_eq!(
            w.accounts
                .login(PROVIDER, identity("s1", "adam", Some(Role::Member)))
                .await
                .unwrap()
                .status,
            UserStatus::Active
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

    async fn owning_everything(w: &World, handle: &str, key: &str) -> User {
        let user = w.member(handle).await;
        w.sandbox(&user, "main").await;
        w.sandbox(&user, "dev").await;
        w.keys.add(&user, "laptop", key).await.unwrap();
        w.credentials
            .set(user.id, &credential("A_KEY"), secret("value"))
            .await
            .unwrap();
        user
    }

    async fn owns_nothing(w: &World, user: &User) {
        assert_eq!(w.users.get(user.id).await.unwrap(), None);
        assert_eq!(w.users.by_handle(&user.handle).await.unwrap(), None);
        assert_eq!(w.sandboxes.list(user).await.unwrap(), []);
        assert_eq!(w.keys.list(user).await.unwrap(), []);
        assert_eq!(w.credentials.names(user.id).await.unwrap(), []);
        assert!(w.credentials.load(user.id).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn deleting_a_user_removes_everything_they_own_even_after_a_failure() {
        let w = World::new();
        let by = admin_proof(&w.admin("root").await);
        let eve = owning_everything(&w, "eve", EVE_KEY).await;
        let adam = owning_everything(&w, "adam", ADAM_KEY).await;
        let eves: Vec<_> = w.sandboxes.list(&eve).await.unwrap();

        w.engine.fail_deletes_after(Some(1));
        assert!(matches!(
            w.accounts.delete_user(&by, eve.id).await,
            Err(Error::Internal { .. })
        ));
        assert!(matches!(
            w.accounts.require_active(eve.id).await,
            Err(Error::Forbidden)
        ));
        assert!(!w.access.authenticate("eve+dev", EVE_KEY).await.unwrap());
        assert_eq!(w.sandboxes.list(&eve).await.unwrap().len(), 1);

        w.engine.fail_deletes_after(None);
        w.accounts.delete_user(&by, eve.id).await.unwrap();
        owns_nothing(&w, &eve).await;
        for view in &eves {
            assert!(matches!(
                w.engine.status(&view.sandbox.id).await,
                Err(Error::NotFound(_))
            ));
        }
        assert_eq!(w.provider.forgotten(), ["eve"]);
        assert!(matches!(
            w.accounts.delete_user(&by, eve.id).await,
            Err(Error::NotFound(_))
        ));

        assert_eq!(w.sandboxes.list(&adam).await.unwrap().len(), 2);
        assert_eq!(w.keys.list(&adam).await.unwrap().len(), 1);
        assert_eq!(w.credentials.names(adam.id).await.unwrap().len(), 1);
        assert!(w.accounts.require_active(adam.id).await.is_ok());
    }

    #[tokio::test]
    async fn deleting_an_identity_deletes_its_user_and_frees_the_username() {
        let w = World::new();
        let by = admin_proof(&w.admin("root").await);
        let eve = owning_everything(&w, "eve", EVE_KEY).await;

        assert!(
            w.accounts
                .delete_identity(&by, PROVIDER, "eve")
                .await
                .unwrap()
        );
        owns_nothing(&w, &eve).await;
        assert_eq!(w.provider.forgotten(), ["eve"]);

        let again = owning_everything(&w, "eve", EVE_KEY).await;
        assert_ne!(again.id, eve.id);
        assert_eq!(again.handle, eve.handle);
        assert!(w.access.authenticate("eve+main", EVE_KEY).await.unwrap());
    }

    #[tokio::test]
    async fn an_identity_that_never_logged_in_is_only_forgotten() {
        let w = World::new();
        let by = admin_proof(&w.admin("root").await);
        let before = w.users.list().await.unwrap();
        assert!(
            !w.accounts
                .delete_identity(&by, PROVIDER, "never")
                .await
                .unwrap()
        );
        assert!(
            !w.accounts
                .delete_identity(&by, "gone", "never")
                .await
                .unwrap()
        );
        assert_eq!(w.provider.forgotten(), ["never"]);
        assert_eq!(w.users.list().await.unwrap(), before);
    }

    #[tokio::test]
    async fn users_delete_themselves() {
        let w = World::new();
        let eve = owning_everything(&w, "eve", EVE_KEY).await;
        w.accounts.delete_self(eve.id).await.unwrap();
        owns_nothing(&w, &eve).await;
        assert_eq!(w.provider.forgotten(), ["eve"]);
    }

    #[tokio::test]
    async fn a_self_deletion_that_failed_is_finished_by_the_same_user() {
        let w = World::new();
        let eve = owning_everything(&w, "eve", EVE_KEY).await;
        w.engine.fail_deletes_after(Some(1));
        assert!(matches!(
            w.accounts.delete_self(eve.id).await,
            Err(Error::Internal { .. })
        ));
        assert_eq!(
            w.users.get(eve.id).await.unwrap().unwrap().status,
            UserStatus::Deleting
        );

        w.engine.fail_deletes_after(None);
        w.accounts.delete_self(eve.id).await.unwrap();
        owns_nothing(&w, &eve).await;
        assert_eq!(w.provider.forgotten(), ["eve"]);
    }

    #[tokio::test]
    async fn a_user_being_deleted_cannot_log_in_ssh_or_be_enabled() {
        let w = World::new();
        let by = admin_proof(&w.admin("root").await);
        let eve = owning_everything(&w, "eve", EVE_KEY).await;
        w.engine.fail_deletes_after(Some(0));
        assert!(w.accounts.delete_user(&by, eve.id).await.is_err());

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
        assert!(!w.access.authenticate("eve+main", EVE_KEY).await.unwrap());
        assert!(matches!(
            w.access.route("eve+main").await,
            Err(Error::Forbidden)
        ));
        for enabled in [true, false] {
            assert!(matches!(
                w.accounts.set_enabled(&by, eve.id, enabled).await,
                Err(Error::Conflict(_))
            ));
        }
        assert!(matches!(
            w.accounts
                .login(PROVIDER, identity("eve", "eve", None))
                .await,
            Err(Error::Forbidden)
        ));
        assert_eq!(
            w.users.get(eve.id).await.unwrap().unwrap().status,
            UserStatus::Deleting
        );
    }

    #[tokio::test]
    async fn a_disabled_user_cannot_delete_themselves_but_an_admin_can() {
        let w = World::new();
        let by = admin_proof(&w.admin("root").await);
        let eve = owning_everything(&w, "eve", EVE_KEY).await;
        w.accounts.set_enabled(&by, eve.id, false).await.unwrap();
        assert!(matches!(
            w.accounts.delete_self(eve.id).await,
            Err(Error::Forbidden)
        ));
        assert_eq!(w.sandboxes.list(&eve).await.unwrap().len(), 2);
        assert_eq!(w.keys.list(&eve).await.unwrap().len(), 1);
        assert_eq!(
            w.users.get(eve.id).await.unwrap().unwrap().status,
            UserStatus::Disabled
        );

        w.accounts.delete_user(&by, eve.id).await.unwrap();
        owns_nothing(&w, &eve).await;
    }
}
