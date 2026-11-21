use async_trait::async_trait;

use crate::Result;
use crate::domain::Principal;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoginState(pub String);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoginStart {
    pub url: String,
    pub state: LoginState,
}

#[async_trait]
pub trait IdentityProvider: Send + Sync {
    async fn begin(&self) -> Result<LoginStart>;
    async fn complete(
        &self,
        state: LoginState,
        code: &str,
        returned_state: &str,
    ) -> Result<Principal>;
}
