use std::net::IpAddr;
use std::sync::Arc;

use cookie::Key;
use corral_core::app::{Accounts, Credentials, Keys, Sandboxes};
use corral_core::ports::{AccountAdmin, Identity, IdentityProvider};
use secrecy::SecretString;
use serde::Serialize;

use crate::error::ApiError;
use crate::login_limiter::LoginLimiter;

pub(crate) type AppState = Arc<Api>;

/// Everything the routes use.
pub struct Api {
    pub accounts: Accounts,
    pub keys: Keys,
    pub credentials: Credentials,
    pub sandboxes: Sandboxes,
    pub providers: Providers,
    pub ssh: SshEndpoint,
    pub session_key: Key,
    /// Marks the cookies `Secure`, for an `https://` base URL.
    pub secure_cookies: bool,
    pub trust_forwarded_for: bool,
}

/// Where people point their SSH client.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SshEndpoint {
    pub host: String,
    pub port: u16,
}

/// The login methods that are on, with one failed-login limit across every password provider.
pub struct Providers {
    providers: Vec<Arc<dyn IdentityProvider>>,
    limiter: LoginLimiter,
}

impl Providers {
    #[must_use]
    pub fn new(providers: Vec<Arc<dyn IdentityProvider>>) -> Self {
        Self {
            providers,
            limiter: LoginLimiter::default(),
        }
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &dyn IdentityProvider> {
        self.providers.iter().map(AsRef::as_ref)
    }

    pub(crate) fn get(&self, id: &str) -> Result<&dyn IdentityProvider, ApiError> {
        self.iter()
            .find(|provider| provider.id() == id)
            .ok_or_else(ApiError::not_found)
    }

    pub(crate) fn account_admin(&self) -> Result<&dyn AccountAdmin, ApiError> {
        self.iter()
            .find_map(|provider| provider.accounts())
            .ok_or_else(ApiError::not_found)
    }

    pub(crate) async fn verify_password(
        &self,
        id: &str,
        username: String,
        password: SecretString,
        address: IpAddr,
    ) -> Result<Identity, ApiError> {
        let provider = self.get(id)?;
        self.limiter
            .verify(provider, username, password, address)
            .await
    }
}
