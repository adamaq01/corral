use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::Result;
use crate::domain::{
    Handle, Sandbox, SandboxDefaults, SandboxId, SandboxName, SshKey, SshPublicKey, TokenHash,
    User, UserId,
};

#[async_trait]
pub trait UserRepo: Send + Sync {
    async fn by_subject(&self, subject: &str) -> Result<Option<User>>;
    async fn by_handle(&self, handle: &Handle) -> Result<Option<User>>;
    async fn get(&self, id: UserId) -> Result<Option<User>>;
    async fn insert(&self, user: &User) -> Result<()>;
    async fn update(&self, user: &User) -> Result<()>;
    async fn list(&self) -> Result<Vec<User>>;
    /// Whether the user existed.
    async fn delete(&self, id: UserId) -> Result<bool>;
    /// When the user last set or removed a credential, if ever.
    async fn credentials_changed_at(&self, id: UserId) -> Result<Option<DateTime<Utc>>>;
    /// Does nothing unless the user exists.
    async fn record_credentials_change(&self, id: UserId, at: DateTime<Utc>) -> Result<()>;
    /// Whether the user lets `corral delete` inside their sandboxes schedule a deletion.
    async fn deletes_from_inside(&self, id: UserId) -> Result<bool>;
    /// Does nothing unless the user exists.
    async fn set_deletes_from_inside(&self, id: UserId, allowed: bool) -> Result<()>;
    /// The defaults the user picked, if they ever did.
    async fn sandbox_defaults(&self, id: UserId) -> Result<Option<SandboxDefaults>>;
    /// Does nothing unless the user exists.
    async fn set_sandbox_defaults(&self, id: UserId, defaults: &SandboxDefaults) -> Result<()>;
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
    /// Refuses with `LimitReached` when the owner already has `max_per_owner` sandboxes, checked
    /// atomically with the insert.
    async fn insert(&self, sandbox: &Sandbox, max_per_owner: usize) -> Result<()>;
    async fn delete(&self, owner: UserId, name: &SandboxName) -> Result<bool>;
    /// The sandbox the owner picked as default, if it still exists.
    async fn default_id(&self, owner: UserId) -> Result<Option<SandboxId>>;
    /// Does nothing unless `id` is one of the owner's sandboxes.
    async fn set_default(&self, owner: UserId, id: &SandboxId) -> Result<()>;
    /// Sets the sandbox's `started_at`, and the token of that start in place of any earlier one.
    /// Does nothing unless the sandbox exists.
    async fn record_start(
        &self,
        id: &SandboxId,
        at: DateTime<Utc>,
        token: &TokenHash,
    ) -> Result<()>;
    /// Leaves the sandbox without a token.
    async fn revoke_token(&self, id: &SandboxId) -> Result<()>;
    async fn by_token(&self, token: &TokenHash) -> Result<Option<Sandbox>>;
    /// Schedules the sandbox's deletion, or with `None` cancels it. Does nothing unless the
    /// sandbox exists.
    async fn set_delete_at(&self, id: &SandboxId, at: Option<DateTime<Utc>>) -> Result<()>;
    /// Every sandbox whose deletion is due at `now`, oldest first.
    async fn due_for_deletion(&self, now: DateTime<Utc>) -> Result<Vec<Sandbox>>;
}
