use async_trait::async_trait;

use crate::Result;
use crate::domain::User;

#[async_trait]
pub trait KeyVerifier: Send + Sync {
    async fn verify(&self, user: &User, presented: &str) -> Result<bool>;
}
