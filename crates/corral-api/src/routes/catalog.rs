use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};

use crate::auth::CurrentUser;
use crate::dto::CatalogDto;
use crate::state::AppState;

pub(crate) fn routes() -> Router<AppState> {
    Router::new().route("/api/catalog", get(catalog))
}

async fn catalog(_: CurrentUser, State(state): State<AppState>) -> Json<CatalogDto> {
    Json(CatalogDto::new(state.sandboxes.catalog(), &state.ssh))
}
