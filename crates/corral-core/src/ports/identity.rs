use std::fmt;

use async_trait::async_trait;

use crate::Result;
use crate::domain::Principal;

#[derive(Clone, PartialEq, Eq)]
pub struct LoginState(pub String);

impl fmt::Debug for LoginState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("LoginState(..)")
    }
}

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

#[cfg(test)]
mod tests {
    use super::{LoginStart, LoginState};

    #[test]
    fn debug_never_prints_the_login_state_payload() {
        let start = LoginStart {
            url: "https://idp.example.com/authorize?state=public".to_owned(),
            state: LoginState("pkce-verifier-and-nonce".to_owned()),
        };
        let printed = format!("{start:?} {:?} {:#?}", start.state, start);
        assert!(!printed.contains("pkce-verifier-and-nonce"), "{printed}");
        assert!(printed.contains("LoginState(..)"));
    }
}
