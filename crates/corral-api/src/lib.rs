mod auth;
mod client_address;
mod dto;
mod error;
mod extract;
mod json_only;
mod login_limiter;
mod routes;
mod session;
mod state;

use std::sync::Arc;

use axum::Router;
use axum::middleware::from_fn;

pub use cookie::Key;
pub use state::{Api, Providers, SshEndpoint};

pub fn router(api: Api) -> Router {
    routes::all()
        .layer(from_fn(json_only::require_json))
        .with_state(Arc::new(api))
}
