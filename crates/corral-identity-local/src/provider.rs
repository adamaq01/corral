use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use corral_core::domain::{Admin, Handle, LocalAccount, Role, User};
use corral_core::ports::{
    AccountAdmin, Answer, Challenge, Identity, IdentityProvider, LocalAccountRepo, Method, UserRepo,
};
use corral_core::{Error, Result};
use secrecy::SecretString;
use uuid::Uuid;

use crate::password;

const ID: &str = "local";

pub struct LocalProvider {
    accounts: Arc<dyn LocalAccountRepo>,
    users: Arc<dyn UserRepo>,
    dummy_hash: String,
}

impl LocalProvider {
    pub fn new(accounts: Arc<dyn LocalAccountRepo>, users: Arc<dyn UserRepo>) -> Result<Self> {
        Ok(Self {
            accounts,
            users,
            dummy_hash: password::hash(&SecretString::from(Uuid::new_v4().to_string()))?,
        })
    }

    /// Adds an account with no admin acting, for the server's command line.
    pub async fn add(&self, username: &Handle, password: SecretString, role: Role) -> Result<()> {
        if self.users.by_subject(&subject(username)).await?.is_some() {
            return Err(Error::Conflict(format!(
                "local account {username} exists or was deleted, and usernames are never reused"
            )));
        }
        self.accounts
            .insert(&LocalAccount {
                username: username.clone(),
                password_hash: password::hash(&password)?,
                role,
                created_at: Utc::now(),
            })
            .await
    }

    async fn find(&self, username: &Handle) -> Result<LocalAccount> {
        self.accounts
            .get(username)
            .await?
            .ok_or_else(|| Error::NotFound(format!("local account {username}")))
    }
}

fn subject(username: &Handle) -> String {
    format!("{ID}:{username}")
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
                subject: subject(&account.username),
                username: account.username.to_string(),
                email: None,
                role: Some(account.role),
            }),
            _ => Err(Error::Forbidden),
        }
    }

    fn accounts(&self) -> Option<&dyn AccountAdmin> {
        Some(self)
    }
}

#[async_trait]
impl AccountAdmin for LocalProvider {
    async fn create(
        &self,
        _: &Admin,
        username: &Handle,
        password: SecretString,
        role: Role,
    ) -> Result<()> {
        self.add(username, password, role).await
    }

    async fn set_password(
        &self,
        _: &Admin,
        username: &Handle,
        password: SecretString,
    ) -> Result<()> {
        let account = self.find(username).await?;
        let password_hash = password::hash(&password)?;
        self.accounts
            .update(&LocalAccount {
                password_hash,
                ..account
            })
            .await
    }

    async fn delete(&self, _: &Admin, username: &Handle) -> Result<()> {
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

    async fn list(&self, _: &Admin) -> Result<Vec<LocalAccount>> {
        self.accounts.list().await
    }
}
