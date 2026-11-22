use std::sync::Arc;

use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::Value;

use crate::session::Session;
use crate::state::AppState;

pub(crate) fn routes(providers: Vec<Value>) -> Router<AppState> {
    let providers = Arc::new(Value::Array(providers));
    Router::new()
        .route(
            "/auth/providers",
            get(move || std::future::ready(Json(Value::clone(&providers)))),
        )
        .route("/auth/logout", post(logout))
}

async fn logout(session: Session) -> impl IntoResponse {
    (session.end(), StatusCode::NO_CONTENT)
}
