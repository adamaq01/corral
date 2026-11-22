use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use corral_core::app::CreateSandbox;
use corral_core::domain::SandboxName;
use corral_core::ports::Readiness;

use crate::auth::CurrentUser;
use crate::dto::{SandboxDto, Wake};
use crate::error::ApiError;
use crate::extract::{JsonBody, PathParam};
use crate::state::AppState;

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/sandboxes", get(list).post(create))
        .route("/api/sandboxes/{name}", get(show).delete(delete))
        .route("/api/sandboxes/{name}/wake", post(wake))
        .route("/api/sandboxes/{name}/stop", post(stop))
        .route("/api/sandboxes/{name}/restart", post(restart))
        .route("/api/sandboxes/{name}/default", post(make_default))
}

async fn list(
    CurrentUser(user): CurrentUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<SandboxDto>>, ApiError> {
    let views = state.sandboxes.list(&user).await?;
    Ok(Json(
        views
            .into_iter()
            .map(|view| SandboxDto::new(view, &state.ssh))
            .collect(),
    ))
}

async fn create(
    CurrentUser(user): CurrentUser,
    State(state): State<AppState>,
    JsonBody(request): JsonBody<CreateSandbox>,
) -> Result<(StatusCode, Json<SandboxDto>), ApiError> {
    let view = state.sandboxes.create(&user, request).await?;
    Ok((StatusCode::CREATED, Json(SandboxDto::new(view, &state.ssh))))
}

async fn show(
    CurrentUser(user): CurrentUser,
    State(state): State<AppState>,
    PathParam(name): PathParam<SandboxName>,
) -> Result<Json<SandboxDto>, ApiError> {
    let view = state.sandboxes.get(&user, &name).await?;
    Ok(Json(SandboxDto::new(view, &state.ssh)))
}

async fn delete(
    CurrentUser(user): CurrentUser,
    State(state): State<AppState>,
    PathParam(name): PathParam<SandboxName>,
) -> Result<StatusCode, ApiError> {
    state.sandboxes.delete(&user, &name).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn wake(
    CurrentUser(user): CurrentUser,
    State(state): State<AppState>,
    PathParam(name): PathParam<SandboxName>,
) -> Result<Json<Wake>, ApiError> {
    let readiness = state.sandboxes.wake(&user, &name).await?;
    Ok(Json(Wake {
        ready: matches!(readiness, Readiness::Ready(_)),
    }))
}

async fn stop(
    CurrentUser(user): CurrentUser,
    State(state): State<AppState>,
    PathParam(name): PathParam<SandboxName>,
) -> Result<StatusCode, ApiError> {
    state.sandboxes.stop(&user, &name).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn restart(
    CurrentUser(user): CurrentUser,
    State(state): State<AppState>,
    PathParam(name): PathParam<SandboxName>,
) -> Result<StatusCode, ApiError> {
    state.sandboxes.restart(&user, &name).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn make_default(
    CurrentUser(user): CurrentUser,
    State(state): State<AppState>,
    PathParam(name): PathParam<SandboxName>,
) -> Result<StatusCode, ApiError> {
    state.sandboxes.set_default(&user, &name).await?;
    Ok(StatusCode::NO_CONTENT)
}
