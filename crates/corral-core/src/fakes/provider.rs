use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::sync::Mutex;

use async_trait::async_trait;
use chrono::Utc;
use secrecy::{ExposeSecret, SecretString};

use super::{PROVIDER, lock};
use crate::domain::{Handle, Role};
use crate::ports::{
    Answer, Challenge, Identity, IdentityProvider, ManagedAccount, ManagedAccounts, Method,
};
use crate::{Error, Result};

/// A password provider named `test` that keeps its accounts in memory, passwords in the clear.
#[derive(Default)]
pub struct FakeProvider {
    accounts: Mutex<BTreeMap<String, (ManagedAccount, String)>>,
}

#[async_trait]
impl IdentityProvider for FakeProvider {
    fn id(&self) -> &str {
        PROVIDER
    }

    fn method(&self) -> Method {
        Method::Password
    }

    async fn challenge(&self) -> Result<Challenge> {
        Ok(Challenge::Password)
    }

    async fn verify(&self, answer: Answer) -> Result<Identity> {
        let Answer::Password { username, password } = answer else {
            return Err(Error::Invalid("a password is needed".to_owned()));
        };
        match lock(&self.accounts).get(&username) {
            Some((account, kept)) if kept == password.expose_secret() => Ok(Identity {
                subject: account.subject.clone(),
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
impl ManagedAccounts for FakeProvider {
    async fn create(&self, username: &Handle, password: SecretString, role: Role) -> Result<()> {
        match lock(&self.accounts).entry(username.to_string()) {
            Entry::Occupied(_) => Err(Error::Conflict(format!("account {username}"))),
            Entry::Vacant(slot) => {
                let account = ManagedAccount {
                    subject: username.to_string(),
                    username: username.clone(),
                    role,
                    created_at: Utc::now(),
                };
                slot.insert((account, password.expose_secret().to_owned()));
                Ok(())
            }
        }
    }

    async fn set_password(&self, subject: &str, password: SecretString) -> Result<()> {
        let mut accounts = lock(&self.accounts);
        let (_, kept) = accounts
            .get_mut(subject)
            .ok_or_else(|| Error::NotFound(format!("account {subject}")))?;
        password.expose_secret().clone_into(kept);
        Ok(())
    }

    async fn delete(&self, subject: &str) -> Result<bool> {
        Ok(lock(&self.accounts).remove(subject).is_some())
    }

    async fn list(&self) -> Result<Vec<ManagedAccount>> {
        Ok(lock(&self.accounts)
            .values()
            .map(|(account, _)| account.clone())
            .collect())
    }
}
