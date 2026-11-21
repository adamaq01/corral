use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{delete, get};
use axum::{Json, Router};
use uuid::Uuid;

use crate::auth::CurrentUser;
use crate::dto::{KeyDto, NewKey};
use crate::error::ApiError;
use crate::extract::{JsonBody, PathParam};
use crate::state::AppState;

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/keys", get(list).post(add))
        .route("/api/keys/{id}", delete(remove))
}

async fn list(
    CurrentUser(user): CurrentUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<KeyDto>>, ApiError> {
    let keys = state.keys.list(&user).await?;
    Ok(Json(keys.into_iter().map(KeyDto::from).collect()))
}

async fn add(
    CurrentUser(user): CurrentUser,
    State(state): State<AppState>,
    JsonBody(key): JsonBody<NewKey>,
) -> Result<(StatusCode, Json<KeyDto>), ApiError> {
    let key = state.keys.add(&user, &key.label, &key.public_key).await?;
    Ok((StatusCode::CREATED, Json(KeyDto::from(key))))
}

async fn remove(
    CurrentUser(user): CurrentUser,
    State(state): State<AppState>,
    PathParam(id): PathParam<Uuid>,
) -> Result<StatusCode, ApiError> {
    state.keys.remove(&user, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
