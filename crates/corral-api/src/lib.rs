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

use axum::Router;
use axum::middleware::from_fn;

pub use cookie::Key;
pub use session::{Session, SessionCodec};
pub use state::{ApiState, SshEndpoint};

pub fn router(state: ApiState) -> Router {
    routes::all()
        .layer(from_fn(json_only::require_json))
        .with_state(state::AppState::new(state))
}
