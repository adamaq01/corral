use async_trait::async_trait;
use secrecy::SecretString;

use super::Env;
use crate::Result;
use crate::domain::{CredentialName, UserId};

#[async_trait]
pub trait SecretStore: Send + Sync {
    async fn set(&self, user: UserId, name: &CredentialName, value: SecretString) -> Result<()>;
    async fn remove(&self, user: UserId, name: &CredentialName) -> Result<bool>;
    async fn names(&self, user: UserId) -> Result<Vec<CredentialName>>;
    async fn load(&self, user: UserId) -> Result<Env>;
}
