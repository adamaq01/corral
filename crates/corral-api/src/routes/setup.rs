use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, put};
use axum::{Json, Router};
use corral_core::domain::{EntryId, SandboxName};
use uuid::Uuid;

use crate::auth::CurrentUser;
use crate::error::ApiError;
use crate::extract::{JsonBody, PathParam};
use crate::setup_dto::{
    AttachedDto, EntryChangeDto, EntryDto, NewEntryDto, PreviewDto, ValueInput,
};
use crate::state::AppState;

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/setup", get(list).post(create))
        .route("/api/setup/{id}", get(show).put(change).delete(remove))
        .route("/api/sandboxes/{name}/setup", get(of_sandbox))
        .route(
            "/api/sandboxes/{name}/setup/{id}",
            put(attach).delete(detach),
        )
        .route(
            "/api/sandboxes/{name}/setup/{id}/override",
            put(set_override).delete(clear_override),
        )
        .route("/api/sandboxes/{name}/setup/{id}/preview", get(preview))
}

async fn list(
    CurrentUser(user): CurrentUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<EntryDto>>, ApiError> {
    let views = state.setup.list(&user).await?;
    Ok(Json(views.into_iter().map(EntryDto::from).collect()))
}

async fn create(
    CurrentUser(user): CurrentUser,
    State(state): State<AppState>,
    JsonBody(body): JsonBody<NewEntryDto>,
) -> Result<(StatusCode, Json<EntryDto>), ApiError> {
    let view = state.setup.create(&user, body.parse()?).await?;
    Ok((StatusCode::CREATED, Json(EntryDto::from(view))))
}

async fn show(
    CurrentUser(user): CurrentUser,
    State(state): State<AppState>,
    PathParam(id): PathParam<Uuid>,
) -> Result<Json<EntryDto>, ApiError> {
    let view = state.setup.get(&user, EntryId(id)).await?;
    Ok(Json(EntryDto::from(view)))
}

async fn change(
    CurrentUser(user): CurrentUser,
    State(state): State<AppState>,
    PathParam(id): PathParam<Uuid>,
    JsonBody(body): JsonBody<EntryChangeDto>,
) -> Result<Json<EntryDto>, ApiError> {
    let view = state
        .setup
        .change(&user, EntryId(id), body.parse()?)
        .await?;
    Ok(Json(EntryDto::from(view)))
}

/// Detaches the entry from every sandbox, which loses it at its next start. Refused while a
/// template that stays refers to it.
async fn remove(
    CurrentUser(user): CurrentUser,
    State(state): State<AppState>,
    PathParam(id): PathParam<Uuid>,
) -> Result<StatusCode, ApiError> {
    state.setup.delete(&user, EntryId(id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn of_sandbox(
    CurrentUser(user): CurrentUser,
    State(state): State<AppState>,
    PathParam(name): PathParam<SandboxName>,
) -> Result<Json<Vec<AttachedDto>>, ApiError> {
    let attached = state.setup.of_sandbox(&user, &name).await?;
    Ok(Json(attached.into_iter().map(AttachedDto::from).collect()))
}

async fn attach(
    CurrentUser(user): CurrentUser,
    State(state): State<AppState>,
    PathParam((name, id)): PathParam<(SandboxName, Uuid)>,
) -> Result<StatusCode, ApiError> {
    state.setup.attach(&user, &name, EntryId(id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn detach(
    CurrentUser(user): CurrentUser,
    State(state): State<AppState>,
    PathParam((name, id)): PathParam<(SandboxName, Uuid)>,
) -> Result<StatusCode, ApiError> {
    state.setup.detach(&user, &name, EntryId(id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn set_override(
    CurrentUser(user): CurrentUser,
    State(state): State<AppState>,
    PathParam((name, id)): PathParam<(SandboxName, Uuid)>,
    JsonBody(value): JsonBody<ValueInput>,
) -> Result<StatusCode, ApiError> {
    state
        .setup
        .set_override(&user, &name, EntryId(id), Some(value.bytes()?))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn clear_override(
    CurrentUser(user): CurrentUser,
    State(state): State<AppState>,
    PathParam((name, id)): PathParam<(SandboxName, Uuid)>,
) -> Result<StatusCode, ApiError> {
    state
        .setup
        .set_override(&user, &name, EntryId(id), None)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn preview(
    CurrentUser(user): CurrentUser,
    State(state): State<AppState>,
    PathParam((name, id)): PathParam<(SandboxName, Uuid)>,
) -> Result<Json<PreviewDto>, ApiError> {
    let preview = state.setup.preview(&user, &name, EntryId(id)).await?;
    Ok(Json(PreviewDto::from(preview)))
}
