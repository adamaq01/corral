use std::ops::Deref;
use std::sync::Arc;

use axum::extract::FromRef;
use cookie::Key;
use corral_core::app::{Accounts, Credentials, Keys, Sandboxes};
use corral_core::ports::{AccountAdmin, IdentityProvider};

use crate::error::ApiError;
use crate::login_limiter::LoginLimiter;
use crate::session::SessionCodec;

pub struct ApiState {
    pub accounts: Accounts,
    pub keys: Keys,
    pub credentials: Credentials,
    pub sandboxes: Sandboxes,
    pub ssh: SshEndpoint,
    pub providers: Vec<Arc<dyn IdentityProvider>>,
    pub sessions: SessionCodec,
    pub trust_forwarded_for: bool,
}

/// Where people point their SSH client.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshEndpoint {
    pub host: String,
    pub port: u16,
}

impl ApiState {
    pub(crate) fn provider(&self, id: &str) -> Result<&dyn IdentityProvider, ApiError> {
        self.providers
            .iter()
            .find(|provider| provider.id() == id)
            .map(AsRef::as_ref)
            .ok_or_else(ApiError::not_found)
    }

    pub(crate) fn account_admin(&self) -> Result<&dyn AccountAdmin, ApiError> {
        self.providers
            .iter()
            .find_map(|provider| provider.accounts())
            .ok_or_else(ApiError::not_found)
    }
}

#[derive(Clone)]
pub(crate) struct AppState {
    api: Arc<ApiState>,
    pub(crate) logins: Arc<LoginLimiter>,
}

impl AppState {
    pub(crate) fn new(api: ApiState) -> Self {
        Self {
            api: Arc::new(api),
            logins: Arc::default(),
        }
    }
}

impl Deref for AppState {
    type Target = ApiState;

    fn deref(&self) -> &ApiState {
        &self.api
    }
}

impl FromRef<AppState> for Key {
    fn from_ref(state: &AppState) -> Self {
        state.sessions.key()
    }
}
