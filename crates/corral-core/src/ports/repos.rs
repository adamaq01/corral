use async_trait::async_trait;
use uuid::Uuid;

use crate::Result;
use crate::domain::{
    CredentialName, Handle, Sandbox, SandboxName, SshKey, SshPublicKey, User, UserId,
};

#[async_trait]
pub trait UserRepo: Send + Sync {
    async fn by_subject(&self, subject: &str) -> Result<Option<User>>;
    async fn by_handle(&self, handle: &Handle) -> Result<Option<User>>;
    async fn get(&self, id: UserId) -> Result<Option<User>>;
    async fn insert(&self, user: &User) -> Result<()>;
    async fn update(&self, user: &User) -> Result<()>;
    async fn list(&self) -> Result<Vec<User>>;
}

#[async_trait]
pub trait KeyRepo: Send + Sync {
    async fn list(&self, user: UserId) -> Result<Vec<SshKey>>;
    async fn insert(&self, key: &SshKey) -> Result<()>;
    async fn delete(&self, user: UserId, id: Uuid) -> Result<bool>;
    async fn by_public_key(&self, public_key: &SshPublicKey) -> Result<Option<SshKey>>;
}

#[async_trait]
pub trait SandboxRepo: Send + Sync {
    async fn list(&self, owner: UserId) -> Result<Vec<Sandbox>>;
    async fn list_all(&self) -> Result<Vec<Sandbox>>;
    async fn by_name(&self, owner: UserId, name: &SandboxName) -> Result<Option<Sandbox>>;
    async fn insert(&self, sandbox: &Sandbox) -> Result<()>;
    async fn update(&self, sandbox: &Sandbox) -> Result<()>;
    async fn delete(&self, owner: UserId, name: &SandboxName) -> Result<bool>;
}

#[async_trait]
pub trait CredentialRepo: Send + Sync {
    async fn list(&self, user: UserId) -> Result<Vec<CredentialName>>;
    async fn upsert(&self, user: UserId, name: &CredentialName) -> Result<()>;
    async fn delete(&self, user: UserId, name: &CredentialName) -> Result<bool>;
}
