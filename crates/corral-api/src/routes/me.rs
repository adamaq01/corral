use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use chrono::Utc;

use crate::auth::CurrentUser;
use crate::dto::Me;
use crate::error::ApiError;
use crate::session::Session;
use crate::state::AppState;

pub(crate) fn routes() -> Router<AppState> {
    Router::new().route("/api/me", get(me).delete(delete_me))
}

async fn delete_me(
    State(state): State<AppState>,
    session: Session,
) -> Result<impl IntoResponse, ApiError> {
    let user = session
        .current(Utc::now())
        .ok_or_else(ApiError::unauthenticated)?;
    state.accounts.delete_self(user).await?;
    Ok((session.end(), StatusCode::NO_CONTENT))
}

async fn me(CurrentUser(user): CurrentUser) -> Json<Me> {
    Json(Me::from(&user))
}
