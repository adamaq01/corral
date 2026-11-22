mod admin;
mod auth;
mod catalog;
mod credentials;
mod keys;
mod me;
mod sandboxes;

use axum::Router;
use serde_json::Value;

use crate::state::AppState;

pub(crate) fn all(providers: Vec<Value>) -> Router<AppState> {
    Router::new()
        .merge(auth::routes(providers))
        .merge(me::routes())
        .merge(catalog::routes())
        .merge(sandboxes::routes())
        .merge(keys::routes())
        .merge(credentials::routes())
        .merge(admin::routes())
}
