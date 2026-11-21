use axum::routing::get;
use axum::{Json, Router};

use crate::auth::CurrentUser;
use crate::dto::Me;
use crate::state::AppState;

pub(crate) fn routes() -> Router<AppState> {
    Router::new().route("/api/me", get(me))
}

async fn me(CurrentUser(user): CurrentUser) -> Json<Me> {
    Json(Me::from(&user))
}
