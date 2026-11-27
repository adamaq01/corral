mod admin;
mod auth;
mod catalog;
mod inside;
mod keys;
mod me;
mod sandboxes;
mod setup;

use axum::Router;
use axum::routing::any;

use crate::error::ApiError;
use crate::provider::ProviderListing;
use crate::state::AppState;

/// Every path under these is the API's, unknown ones included.
const PREFIXES: [&str; 2] = ["/api", "/auth"];

/// Whether the log shows requests to `path` only at debug: those outside the API, such as the
/// web UI's files and probes, and a sandbox's activity reports.
pub(crate) fn is_quiet(path: &str) -> bool {
    let api = PREFIXES.iter().any(|prefix| {
        path.strip_prefix(prefix)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
    });
    !api || path == inside::ACTIVITY
}

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
        .merge(setup::routes())
        .merge(admin::routes())
}

async fn not_found() -> ApiError {
    ApiError::not_found()
}
