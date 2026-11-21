use std::collections::BTreeMap;
use std::sync::Mutex;

use async_trait::async_trait;
use secrecy::SecretString;

use super::lock;
use crate::Result;
use crate::domain::{CredentialName, UserId};
use crate::ports::{Env, SecretBlobRepo, SecretStore};

#[derive(Default)]
pub struct MemorySecretStore {
    values: Mutex<BTreeMap<(UserId, CredentialName), SecretString>>,
}

#[derive(Default)]
pub struct MemorySecretBlobRepo {
    blobs: Mutex<BTreeMap<(UserId, CredentialName), Vec<u8>>>,
}

#[async_trait]
impl SecretStore for MemorySecretStore {
    async fn put(&self, user: UserId, name: &CredentialName, value: SecretString) -> Result<()> {
        lock(&self.values).insert((user, name.clone()), value);
        Ok(())
    }

    async fn delete(&self, user: UserId, name: &CredentialName) -> Result<()> {
        lock(&self.values).remove(&(user, name.clone()));
        Ok(())
    }

    async fn load_all(&self, user: UserId) -> Result<Env> {
        Ok(lock(&self.values)
            .iter()
            .filter(|((owner, _), _)| *owner == user)
            .map(|((_, name), value)| (name.clone(), value.clone()))
            .collect())
    }
}

#[async_trait]
impl SecretBlobRepo for MemorySecretBlobRepo {
    async fn put(&self, user: UserId, name: &CredentialName, blob: Vec<u8>) -> Result<()> {
        lock(&self.blobs).insert((user, name.clone()), blob);
        Ok(())
    }

    async fn delete(&self, user: UserId, name: &CredentialName) -> Result<()> {
        lock(&self.blobs).remove(&(user, name.clone()));
        Ok(())
    }

    async fn load_all(&self, user: UserId) -> Result<Vec<(CredentialName, Vec<u8>)>> {
        Ok(lock(&self.blobs)
            .iter()
            .filter(|((owner, _), _)| *owner == user)
            .map(|((_, name), blob)| (name.clone(), blob.clone()))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::{MemorySecretBlobRepo, MemorySecretStore};
    use crate::contract::{repos, secrets};
    use crate::fakes::MemoryUserRepo;

    #[tokio::test]
    async fn memory_secret_blob_repo_meets_the_contract() {
        repos::secret_blob_repo(&MemoryUserRepo::default(), &MemorySecretBlobRepo::default()).await;
    }

    #[tokio::test]
    async fn memory_secret_store_meets_the_contract() {
        secrets::secret_store(&MemoryUserRepo::default(), &MemorySecretStore::default()).await;
    }
}
