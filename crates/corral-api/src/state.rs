use std::sync::Arc;

use axum::http::request::Parts;
use cookie::Key;
use corral_core::app::{Accounts, Credentials, Keys, Sandboxes};
use serde::Serialize;
use ts_rs::TS;

use crate::error::ApiError;

pub(crate) type AppState = Arc<Api>;

/// Everything the routes use.
pub struct Api {
    pub accounts: Accounts,
    pub keys: Keys,
    pub credentials: Credentials,
    pub sandboxes: Sandboxes,
    pub ssh: SshEndpoint,
    /// Encrypts the session cookie and providers' cookies.
    pub session_key: Key,
    /// Marks the cookies `Secure`, for an `https://` base URL.
    pub secure_cookies: bool,
    pub trust_forwarded_for: bool,
}

/// Where people point their SSH client.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct SshEndpoint {
    pub host: String,
    pub port: u16,
}

/// The API, for extractors that also serve providers' routes, whose state is their own.
pub(crate) fn shared(parts: &Parts) -> Result<AppState, ApiError> {
    parts.extensions.get::<AppState>().cloned().ok_or_else(|| {
        ApiError::internal(&"no API state: serve the routes through corral_api::router")
    })
}
