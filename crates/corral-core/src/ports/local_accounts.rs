use async_trait::async_trait;

use crate::Result;
use crate::domain::{Handle, LocalAccount};

#[async_trait]
pub trait LocalAccountRepo: Send + Sync {
    async fn get(&self, username: &Handle) -> Result<Option<LocalAccount>>;
    async fn insert(&self, account: &LocalAccount) -> Result<()>;
    async fn update(&self, account: &LocalAccount) -> Result<()>;
    async fn delete(&self, username: &Handle) -> Result<bool>;
    async fn list(&self) -> Result<Vec<LocalAccount>>;
}
