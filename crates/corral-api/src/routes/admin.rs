use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use corral_core::domain::UserId;
use uuid::Uuid;

use crate::auth::CurrentAdmin;
use crate::dto::{OwnedSandbox, UserDto};
use crate::error::ApiError;
use crate::extract::PathParam;
use crate::state::AppState;

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/admin/users", get(users))
        .route("/api/admin/users/{id}", delete(delete_user))
        .route("/api/admin/users/{id}/enable", post(enable))
        .route("/api/admin/users/{id}/disable", post(disable))
        .route("/api/admin/sandboxes", get(sandboxes))
}

async fn users(
    CurrentAdmin(admin): CurrentAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<UserDto>>, ApiError> {
    let users = state.accounts.list(&admin).await?;
    Ok(Json(users.into_iter().map(UserDto::from).collect()))
}

async fn enable(
    CurrentAdmin(admin): CurrentAdmin,
    State(state): State<AppState>,
    PathParam(id): PathParam<Uuid>,
) -> Result<Json<UserDto>, ApiError> {
    let user = state.accounts.set_enabled(&admin, UserId(id), true).await?;
    Ok(Json(UserDto::from(user)))
}

async fn disable(
    CurrentAdmin(admin): CurrentAdmin,
    State(state): State<AppState>,
    PathParam(id): PathParam<Uuid>,
) -> Result<Json<UserDto>, ApiError> {
    let user = state
        .accounts
        .set_enabled(&admin, UserId(id), false)
        .await?;
    Ok(Json(UserDto::from(user)))
}

async fn delete_user(
    CurrentAdmin(admin): CurrentAdmin,
    State(state): State<AppState>,
    PathParam(id): PathParam<Uuid>,
) -> Result<StatusCode, ApiError> {
    state.accounts.delete_user(&admin, UserId(id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn sandboxes(
    CurrentAdmin(admin): CurrentAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<OwnedSandbox>>, ApiError> {
    let views = state.sandboxes.list_all(&admin).await?;
    Ok(Json(
        views
            .into_iter()
            .map(|view| OwnedSandbox::new(view, &state.ssh))
            .collect(),
    ))
}
