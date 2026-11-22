use std::collections::BTreeMap;
use std::sync::Mutex;

use async_trait::async_trait;
use secrecy::SecretString;

use super::lock;
use crate::Result;
use crate::domain::{CredentialName, UserId};
use crate::ports::{Env, SecretStore};

#[derive(Default)]
pub struct MemorySecretStore {
    values: Mutex<BTreeMap<UserId, Env>>,
}

#[async_trait]
impl SecretStore for MemorySecretStore {
    async fn set(&self, user: UserId, name: &CredentialName, value: SecretString) -> Result<()> {
        lock(&self.values)
            .entry(user)
            .or_default()
            .insert(name.clone(), value);
        Ok(())
    }

    async fn remove(&self, user: UserId, name: &CredentialName) -> Result<bool> {
        Ok(lock(&self.values)
            .get_mut(&user)
            .is_some_and(|env| env.remove(name).is_some()))
    }

    async fn names(&self, user: UserId) -> Result<Vec<CredentialName>> {
        Ok(self.load(user).await?.into_keys().collect())
    }

    async fn load(&self, user: UserId) -> Result<Env> {
        Ok(lock(&self.values).get(&user).cloned().unwrap_or_default())
    }
}
