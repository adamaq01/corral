mod auth;
mod client_address;
mod cookies;
mod dto;
mod error;
mod extract;
mod json_only;
mod provider;
mod routes;
mod session;
mod state;
#[cfg(feature = "testing")]
#[allow(clippy::missing_panics_doc)]
pub mod testing;

use std::sync::Arc;

use axum::middleware::from_fn;
use axum::{Extension, Router};

pub use auth::CurrentAdmin;
pub use client_address::ClientAddress;
pub use cookie::Key;
pub use error::ApiError;
pub use extract::{JsonBody, PathParam, QueryParams};
pub use provider::{
    Identities, LoginKind, ProviderCookies, ProviderDescription, ProviderRoutes, SIGN_IN_PAGE,
    SignedIn, provider_path, return_path, sign_in_failed,
};
pub use state::{Api, SshEndpoint};

/// The API with each provider's routes under `provider_path` of its id.
pub fn router(api: Api, providers: Vec<ProviderRoutes>) -> Router {
    let api = Arc::new(api);
    let listed = providers.iter().map(ProviderRoutes::listing).collect();
    let mut router = routes::all(listed).with_state(api.clone());
    for provider in providers {
        let path = provider_path(&provider.id);
        router = router.nest(&path, provider.mounted());
    }
    router
        .layer(from_fn(json_only::require_json))
        .layer(Extension(api))
}
