use corral_core::ports::PendingLogin;
use corral_core::{Error, Result};
use openidconnect::{CsrfToken, Nonce, PkceCodeVerifier};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub(crate) struct LoginSecrets {
    pub csrf: CsrfToken,
    pub nonce: Nonce,
    pub pkce_verifier: PkceCodeVerifier,
}

impl LoginSecrets {
    pub fn encode(&self) -> Result<PendingLogin> {
        serde_json::to_string(self)
            .map(PendingLogin)
            .map_err(|err| Error::internal("encoding login state", err))
    }

    pub fn decode(pending: &PendingLogin) -> Result<Self> {
        serde_json::from_str(&pending.0).map_err(|_| Error::Forbidden)
    }
}
