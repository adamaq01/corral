use std::collections::BTreeMap;

use async_trait::async_trait;
use secrecy::SecretSlice;

use super::VariableMap;
use crate::Result;
use crate::domain::{SecretId, UserId};

/// Keeps the values of users' secret entries as bytes, each under an id core gives it.
#[async_trait]
pub trait SecretStore: Send + Sync {
    /// Keeps `value` under `id`, replacing any value there.
    async fn put(&self, owner: UserId, id: SecretId, value: &SecretSlice<u8>) -> Result<()>;
    /// The values kept under `ids`; an id without one is left out.
    async fn get(
        &self,
        owner: UserId,
        ids: &[SecretId],
    ) -> Result<BTreeMap<SecretId, SecretSlice<u8>>>;
    /// Whether there was a value to remove.
    async fn remove(&self, owner: UserId, id: SecretId) -> Result<bool>;
    /// Removes every value of the owner, the credentials they kept before entries included.
    async fn remove_all(&self, owner: UserId) -> Result<()>;
    /// What the owner kept as credentials, by name, before entries replaced them.
    async fn legacy_credentials(&self, owner: UserId) -> Result<VariableMap>;
    /// Removes those credentials once core has made them entries. Safe to repeat.
    async fn forget_legacy_credentials(&self, owner: UserId) -> Result<()>;
}
