mod admin;
mod auth;
mod catalog;
mod credentials;
mod inside;
mod keys;
mod me;
mod sandboxes;

use axum::Router;
use axum::routing::any;

use crate::error::ApiError;
use crate::provider::ProviderListing;
use crate::state::AppState;

/// Every path under these is the API's, unknown ones included.
const PREFIXES: [&str; 2] = ["/api", "/auth"];

pub(crate) fn all(providers: Vec<ProviderListing>) -> Router<AppState> {
    let mut router = Router::new();
    for prefix in PREFIXES {
        for path in [
            prefix,
            &format!("{prefix}/"),
            &format!("{prefix}/{{*path}}"),
        ] {
            router = router.route(path, any(not_found));
        }
    }
    router
        .merge(auth::routes(providers))
        .merge(me::routes())
        .merge(catalog::routes())
        .merge(sandboxes::routes())
        .merge(inside::routes())
        .merge(keys::routes())
        .merge(credentials::routes())
        .merge(admin::routes())
}

async fn not_found() -> ApiError {
    ApiError::not_found()
}
