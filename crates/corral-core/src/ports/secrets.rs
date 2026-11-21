use async_trait::async_trait;
use secrecy::SecretString;

use super::Env;
use crate::Result;
use crate::domain::{CredentialName, UserId};

#[async_trait]
pub trait SecretStore: Send + Sync {
    async fn put(&self, user: UserId, name: &CredentialName, value: SecretString) -> Result<()>;
    async fn delete(&self, user: UserId, name: &CredentialName) -> Result<()>;
    async fn load_all(&self, user: UserId) -> Result<Env>;
}

#[async_trait]
pub trait SecretBlobRepo: Send + Sync {
    async fn put(&self, user: UserId, name: &CredentialName, blob: Vec<u8>) -> Result<()>;
    async fn delete(&self, user: UserId, name: &CredentialName) -> Result<()>;
    async fn load_all(&self, user: UserId) -> Result<Vec<(CredentialName, Vec<u8>)>>;
}
