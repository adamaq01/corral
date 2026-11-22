use async_trait::async_trait;
use chrono::Utc;
use corral_core::domain::{Handle, Role};
use corral_core::ports::{
    Answer, Challenge, Identity, IdentityProvider, ManagedAccount, ManagedAccounts, Method,
};
use corral_core::{Error, Result};
use corral_sql::Pool;
use secrecy::SecretString;
use uuid::Uuid;

use crate::accounts::{Account, AccountTable};
use crate::password;

const ID: &str = "local";

pub struct LocalProvider {
    accounts: AccountTable,
    dummy_hash: String,
}

impl LocalProvider {
    pub async fn open(pool: Pool) -> Result<Self> {
        Ok(Self {
            accounts: AccountTable::open(pool).await?,
            dummy_hash: password::hash(&SecretString::from(Uuid::new_v4().to_string()))?,
        })
    }
}

fn username(subject: &str) -> Result<Handle> {
    Handle::parse(subject).map_err(|_| Error::NotFound(format!("local account {subject}")))
}

#[async_trait]
impl IdentityProvider for LocalProvider {
    fn id(&self) -> &str {
        ID
    }

    fn method(&self) -> Method {
        Method::Password
    }

    async fn challenge(&self) -> Result<Challenge> {
        Ok(Challenge::Password)
    }

    async fn verify(&self, answer: Answer) -> Result<Identity> {
        let Answer::Password { username, password } = answer else {
            return Err(Error::Invalid(
                "local accounts log in with a password".to_owned(),
            ));
        };
        let account = match Handle::parse(&username) {
            Ok(username) => self.accounts.get(&username).await?,
            Err(_) => None,
        };
        let hash = account
            .as_ref()
            .map_or(&self.dummy_hash, |account| &account.password_hash);
        let verified = password::verify(&password, hash);
        match account {
            Some(account) if verified => Ok(Identity {
                subject: account.username.to_string(),
                username: account.username.to_string(),
                email: None,
                role: Some(account.role),
            }),
            _ => Err(Error::Forbidden),
        }
    }

    fn accounts(&self) -> Option<&dyn ManagedAccounts> {
        Some(self)
    }
}

#[async_trait]
impl ManagedAccounts for LocalProvider {
    async fn create(&self, username: &Handle, password: SecretString, role: Role) -> Result<()> {
        let account = Account {
            username: username.clone(),
            password_hash: password::hash(&password)?,
            role,
            created_at: Utc::now(),
        };
        self.accounts.insert(&account).await
    }

    async fn set_password(&self, subject: &str, password: SecretString) -> Result<()> {
        let username = username(subject)?;
        let hash = password::hash(&password)?;
        if self.accounts.set_password_hash(&username, &hash).await? {
            Ok(())
        } else {
            Err(Error::NotFound(format!("local account {username}")))
        }
    }

    async fn delete(&self, subject: &str) -> Result<bool> {
        match Handle::parse(subject) {
            Ok(username) => self.accounts.delete(&username).await,
            Err(_) => Ok(false),
        }
    }

    async fn list(&self) -> Result<Vec<ManagedAccount>> {
        let accounts = self.accounts.list().await?;
        Ok(accounts
            .into_iter()
            .map(|account| ManagedAccount {
                subject: account.username.to_string(),
                username: account.username,
                role: account.role,
                created_at: account.created_at,
            })
            .collect())
    }
}
