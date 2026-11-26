use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};

use crate::auth::CurrentUser;
use crate::dto::CatalogDto;
use crate::error::ApiError;
use crate::state::AppState;

pub(crate) fn routes() -> Router<AppState> {
    Router::new().route("/api/catalog", get(catalog))
}

async fn catalog(
    CurrentUser(user): CurrentUser,
    State(state): State<AppState>,
) -> Result<Json<CatalogDto>, ApiError> {
    Ok(Json(CatalogDto::new(
        &state.sandboxes.catalog().await?,
        state.sandboxes.defaults(&user).await?,
        state.sandboxes.max_sandboxes_for(&user).await?,
        &state.ssh,
    )))
}
