use std::ops::Deref;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::FromRef;
use cookie::Key;
use corral_core::app::{Accounts, Catalog, Credentials, Keys, LocalAccounts, Sandboxes};
use corral_core::ports::IdentityProvider;

use crate::login_limiter::LoginLimiter;
use crate::session::SessionCodec;

pub struct ApiState {
    pub accounts: Accounts,
    pub keys: Keys,
    pub credentials: Credentials,
    pub sandboxes: Sandboxes,
    pub catalog: Catalog,
    pub identity: Option<Arc<dyn IdentityProvider>>,
    pub local: Option<LocalAccounts>,
    pub sessions: SessionCodec,
    pub wake_deadline: Duration,
    pub trust_forwarded_for: bool,
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
