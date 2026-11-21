mod admin;
mod auth;
mod catalog;
mod credentials;
mod keys;
mod me;
mod sandboxes;

use axum::Router;
use corral_core::app::LocalAccounts;

use crate::error::ApiError;
use crate::state::AppState;

pub(crate) fn all() -> Router<AppState> {
    Router::new()
        .merge(auth::routes())
        .merge(me::routes())
        .merge(catalog::routes())
        .merge(sandboxes::routes())
        .merge(keys::routes())
        .merge(credentials::routes())
        .merge(admin::routes())
}

fn local_accounts(state: &AppState) -> Result<&LocalAccounts, ApiError> {
    state.local.as_ref().ok_or_else(ApiError::not_found)
}
