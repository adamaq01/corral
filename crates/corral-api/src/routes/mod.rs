mod admin;
mod auth;
mod catalog;
mod credentials;
mod keys;
mod me;
mod sandboxes;

use axum::Router;

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
