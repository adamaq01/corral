use corral_core::ports::LoginState;
use corral_core::{Error, Result};
use openidconnect::{CsrfToken, Nonce, PkceCodeVerifier};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub(crate) struct PendingLogin {
    pub csrf: CsrfToken,
    pub nonce: Nonce,
    pub pkce_verifier: PkceCodeVerifier,
}

impl PendingLogin {
    pub fn encode(&self) -> Result<LoginState> {
        serde_json::to_string(self)
            .map(LoginState)
            .map_err(|err| Error::internal("encoding login state", err))
    }

    pub fn decode(state: &LoginState) -> Result<Self> {
        serde_json::from_str(&state.0).map_err(|_| Error::Forbidden)
    }
}
