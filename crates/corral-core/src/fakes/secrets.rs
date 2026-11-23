use std::collections::BTreeMap;
use std::sync::Mutex;

use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretSlice, SecretString};

use super::lock;
use crate::Result;
use crate::domain::{CredentialName, SecretId, UserId};
use crate::ports::{CredentialMap, SecretStore};

#[derive(Default)]
pub struct MemorySecretStore {
    values: Mutex<BTreeMap<(UserId, SecretId), Vec<u8>>>,
    legacy: Mutex<BTreeMap<UserId, CredentialMap>>,
}

impl MemorySecretStore {
    /// Keeps a credential the way stores kept them before entries existed.
    pub fn keep_legacy(&self, owner: UserId, name: &CredentialName, value: &str) {
        lock(&self.legacy)
            .entry(owner)
            .or_default()
            .insert(name.clone(), SecretString::from(value));
    }

    /// How many values the store keeps, of every owner.
    #[must_use]
    pub fn count(&self) -> usize {
        lock(&self.values).len()
    }
}

#[async_trait]
impl SecretStore for MemorySecretStore {
    async fn put(&self, owner: UserId, id: SecretId, value: &SecretSlice<u8>) -> Result<()> {
        lock(&self.values).insert((owner, id), value.expose_secret().to_vec());
        Ok(())
    }

    async fn get(
        &self,
        owner: UserId,
        ids: &[SecretId],
    ) -> Result<BTreeMap<SecretId, SecretSlice<u8>>> {
        let values = lock(&self.values);
        Ok(ids
            .iter()
            .filter_map(|id| {
                let value = values.get(&(owner, *id))?;
                Some((*id, SecretSlice::from(value.clone())))
            })
            .collect())
    }

    async fn remove(&self, owner: UserId, id: SecretId) -> Result<bool> {
        Ok(lock(&self.values).remove(&(owner, id)).is_some())
    }

    async fn remove_all(&self, owner: UserId) -> Result<()> {
        lock(&self.values).retain(|(held, _), _| *held != owner);
        lock(&self.legacy).remove(&owner);
        Ok(())
    }

    async fn legacy_credentials(&self, owner: UserId) -> Result<CredentialMap> {
        Ok(lock(&self.legacy).get(&owner).cloned().unwrap_or_default())
    }

    async fn forget_legacy_credentials(&self, owner: UserId) -> Result<()> {
        lock(&self.legacy).remove(&owner);
        Ok(())
    }
}
