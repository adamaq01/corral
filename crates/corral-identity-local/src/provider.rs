use async_trait::async_trait;
use chrono::Utc;
use corral_core::domain::{Handle, Role};
use corral_core::ports::{Identity, IdentityProvider};
use corral_core::{Error, Result};
use corral_sql::Pool;
use secrecy::SecretString;
use uuid::Uuid;

use crate::accounts::{Account, AccountTable};
use crate::login_limiter::LoginLimiter;
use crate::password;

/// Accounts with a password and a role, kept in the database, each logging in as the subject
/// that is its username.
pub struct LocalProvider {
    pub(crate) accounts: AccountTable,
    pub(crate) limiter: LoginLimiter,
    dummy_hash: String,
}

impl LocalProvider {
    pub const ID: &str = "local";

    pub async fn open(pool: Pool) -> Result<Self> {
        Ok(Self {
            accounts: AccountTable::open(pool).await?,
            limiter: LoginLimiter::default(),
            dummy_hash: password::hash(&SecretString::from(Uuid::new_v4().to_string()))?,
        })
    }

    /// `Conflict` when the username is taken, `Invalid` for a short password.
    pub async fn create(
        &self,
        username: &Handle,
        password: SecretString,
        role: Role,
    ) -> Result<()> {
        let account = Account {
            username: username.clone(),
            password_hash: password::hash(&password)?,
            role,
            created_at: Utc::now(),
        };
        self.accounts.insert(&account).await
    }

    /// `Forbidden` for a wrong password or an unknown username, after the same work either way.
    pub(crate) async fn verify(&self, username: &str, password: &SecretString) -> Result<Identity> {
        let account = match Handle::parse(username) {
            Ok(username) => self.accounts.get(&username).await?,
            Err(_) => None,
        };
        let hash = account
            .as_ref()
            .map_or(&self.dummy_hash, |account| &account.password_hash);
        let verified = password::verify(password, hash);
        match account {
            Some(account) if verified => Ok(Identity {
                subject: account.username.to_string(),
                username: account.username.to_string(),
                email: None,
                picture: None,
                role: Some(account.role),
            }),
            _ => Err(Error::Forbidden),
        }
    }

    /// `NotFound` when no account has the username.
    pub(crate) async fn set_password(&self, username: &str, password: SecretString) -> Result<()> {
        let missing = || Error::NotFound(format!("local account {username}"));
        let handle = Handle::parse(username).map_err(|_| missing())?;
        let hash = password::hash(&password)?;
        if self.accounts.set_password_hash(&handle, &hash).await? {
            Ok(())
        } else {
            Err(missing())
        }
    }

    pub(crate) async fn exists(&self, username: &str) -> Result<bool> {
        match Handle::parse(username) {
            Ok(username) => Ok(self.accounts.get(&username).await?.is_some()),
            Err(_) => Ok(false),
        }
    }
}

#[async_trait]
impl IdentityProvider for LocalProvider {
    fn id(&self) -> &str {
        Self::ID
    }

    async fn forget(&self, subject: &str) -> Result<()> {
        if let Ok(username) = Handle::parse(subject) {
            self.accounts.delete(&username).await?;
        }
        Ok(())
    }
}
