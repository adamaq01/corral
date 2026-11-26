use async_trait::async_trait;

use crate::Result;
use crate::app::{Allowance, CatalogChange, LimitsChange, Registry};
use crate::domain::UserId;

/// The database part of settings, which the admin edits. Entries read back are
/// `Source::Database`.
#[async_trait]
pub trait SettingsRepo: Send + Sync {
    /// `None` until the admin first saves the catalog.
    async fn catalog(&self) -> Result<Option<CatalogChange>>;
    async fn set_catalog(&self, change: &CatalogChange) -> Result<()>;
    async fn limits(&self) -> Result<LimitsChange>;
    async fn set_limits(&self, limits: &LimitsChange) -> Result<()>;
    async fn registries(&self) -> Result<Vec<Registry>>;
    async fn set_registries(&self, registries: &[Registry]) -> Result<()>;
    /// Gone once the user is deleted.
    async fn allowance(&self, user: UserId) -> Result<Option<Allowance>>;
    /// With `None`, removes it. Does nothing unless the user exists.
    async fn set_allowance(&self, user: UserId, allowance: Option<&Allowance>) -> Result<()>;
}

#[async_trait]
pub trait RunningLimit: Send + Sync {
    async fn max_running(&self) -> Result<usize>;
}
