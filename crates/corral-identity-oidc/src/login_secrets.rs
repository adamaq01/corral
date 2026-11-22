use openidconnect::{CsrfToken, Nonce, PkceCodeVerifier};
use serde::{Deserialize, Serialize};

/// What the callback checks the issuer's answer against, and where it sends the browser once
/// signed in, kept in an encrypted cookie.
#[derive(Serialize, Deserialize)]
pub(crate) struct LoginSecrets {
    pub csrf: CsrfToken,
    pub nonce: Nonce,
    pub pkce_verifier: PkceCodeVerifier,
    pub return_to: String,
}
