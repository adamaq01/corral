use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, put};
use axum::{Json, Router};
use corral_core::domain::CredentialName;

use crate::auth::CurrentUser;
use crate::dto::{CredentialDto, CredentialValue};
use crate::error::ApiError;
use crate::extract::{JsonBody, PathParam};
use crate::state::AppState;

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/credentials", get(list))
        .route("/api/credentials/{name}", put(set).delete(remove))
}

async fn list(
    CurrentUser(user): CurrentUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<CredentialDto>>, ApiError> {
    let names = state.credentials.list(&user).await?;
    Ok(Json(names.into_iter().map(CredentialDto::from).collect()))
}

async fn set(
    CurrentUser(user): CurrentUser,
    State(state): State<AppState>,
    PathParam(name): PathParam<CredentialName>,
    JsonBody(body): JsonBody<CredentialValue>,
) -> Result<StatusCode, ApiError> {
    state.credentials.set(&user, &name, body.value).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn remove(
    CurrentUser(user): CurrentUser,
    State(state): State<AppState>,
    PathParam(name): PathParam<CredentialName>,
) -> Result<StatusCode, ApiError> {
    state.credentials.remove(&user, &name).await?;
    Ok(StatusCode::NO_CONTENT)
}
