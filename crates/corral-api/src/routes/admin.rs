use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use corral_core::app::{Allowance, Registry};
use corral_core::domain::{Handle, SandboxName, User, UserId};
use uuid::Uuid;

use crate::auth::CurrentAdmin;
use crate::dto::{
    AllowanceChange, CatalogChangeDto, CatalogSettings, LimitsChangeDto, LimitsSettings,
    OwnedSandbox, RegistriesSettings, RegistryChangeDto, SandboxDetailsDto, UserDto,
};
use crate::error::ApiError;
use crate::extract::{JsonBody, PathParam};
use crate::state::AppState;

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/admin/users", get(users))
        .route("/api/admin/users/{id}", delete(delete_user))
        .route("/api/admin/users/{id}/enable", post(enable))
        .route("/api/admin/users/{id}/disable", post(disable))
        .route("/api/admin/users/{id}/allowance", put(set_allowance))
        .route("/api/admin/sandboxes", get(sandboxes))
        .route(
            "/api/admin/sandboxes/{handle}/{name}",
            get(sandbox).delete(delete_sandbox),
        )
        .route("/api/admin/sandboxes/{handle}/{name}/stop", post(stop))
        .route(
            "/api/admin/sandboxes/{handle}/{name}/restart",
            post(restart),
        )
        .route(
            "/api/admin/settings/catalog",
            get(catalog_settings).put(set_catalog),
        )
        .route(
            "/api/admin/settings/limits",
            get(limits_settings).put(set_limits),
        )
        .route(
            "/api/admin/settings/registries",
            get(registry_settings).put(set_registries),
        )
}

async fn users(
    CurrentAdmin(admin): CurrentAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<UserDto>>, ApiError> {
    let mut users = Vec::new();
    for user in state.accounts.list(&admin).await? {
        users.push(with_allowance(&state, user).await?);
    }
    Ok(Json(users))
}

async fn enable(
    CurrentAdmin(admin): CurrentAdmin,
    State(state): State<AppState>,
    PathParam(id): PathParam<Uuid>,
) -> Result<Json<UserDto>, ApiError> {
    let user = state.accounts.set_enabled(&admin, UserId(id), true).await?;
    Ok(Json(with_allowance(&state, user).await?))
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
    Ok(Json(with_allowance(&state, user).await?))
}

async fn set_allowance(
    CurrentAdmin(admin): CurrentAdmin,
    State(state): State<AppState>,
    PathParam(id): PathParam<Uuid>,
    JsonBody(change): JsonBody<AllowanceChange>,
) -> Result<Json<UserDto>, ApiError> {
    let allowance = Option::<Allowance>::from(change);
    let user = state
        .settings
        .set_allowance(&admin, UserId(id), allowance)
        .await?;
    Ok(Json(UserDto::new(user, allowance)))
}

async fn with_allowance(state: &AppState, user: User) -> Result<UserDto, ApiError> {
    let allowance = state.settings.allowance(user.id).await?;
    Ok(UserDto::new(user, allowance))
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

async fn sandbox(
    CurrentAdmin(admin): CurrentAdmin,
    State(state): State<AppState>,
    PathParam((handle, name)): PathParam<(Handle, SandboxName)>,
) -> Result<Json<SandboxDetailsDto>, ApiError> {
    let details = state.sandboxes.inspect(&admin, &handle, &name).await?;
    Ok(Json(SandboxDetailsDto::new(details, &state.ssh)))
}

async fn stop(
    CurrentAdmin(admin): CurrentAdmin,
    State(state): State<AppState>,
    PathParam((handle, name)): PathParam<(Handle, SandboxName)>,
) -> Result<StatusCode, ApiError> {
    let owner = state.sandboxes.owner(&admin, &handle).await?;
    state.sandboxes.stop(&owner, &name).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn restart(
    CurrentAdmin(admin): CurrentAdmin,
    State(state): State<AppState>,
    PathParam((handle, name)): PathParam<(Handle, SandboxName)>,
) -> Result<StatusCode, ApiError> {
    let owner = state.sandboxes.owner(&admin, &handle).await?;
    state.sandboxes.restart(&owner, &name).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_sandbox(
    CurrentAdmin(admin): CurrentAdmin,
    State(state): State<AppState>,
    PathParam((handle, name)): PathParam<(Handle, SandboxName)>,
) -> Result<StatusCode, ApiError> {
    let owner = state.sandboxes.owner(&admin, &handle).await?;
    state.sandboxes.delete(&owner, &name).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn catalog_settings(
    CurrentAdmin(_): CurrentAdmin,
    State(state): State<AppState>,
) -> Result<Json<CatalogSettings>, ApiError> {
    let view = state.settings.catalog_view().await?;
    Ok(Json(CatalogSettings::from(view)))
}

async fn set_catalog(
    CurrentAdmin(admin): CurrentAdmin,
    State(state): State<AppState>,
    JsonBody(change): JsonBody<CatalogChangeDto>,
) -> Result<StatusCode, ApiError> {
    state
        .settings
        .set_catalog(&admin, change.try_into()?)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn limits_settings(
    CurrentAdmin(_): CurrentAdmin,
    State(state): State<AppState>,
) -> Result<Json<LimitsSettings>, ApiError> {
    let view = state.settings.limits_view().await?;
    Ok(Json(LimitsSettings::from(view)))
}

async fn set_limits(
    CurrentAdmin(admin): CurrentAdmin,
    State(state): State<AppState>,
    JsonBody(change): JsonBody<LimitsChangeDto>,
) -> Result<StatusCode, ApiError> {
    state.settings.set_limits(&admin, change.into()).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn registry_settings(
    CurrentAdmin(_): CurrentAdmin,
    State(state): State<AppState>,
) -> Result<Json<RegistriesSettings>, ApiError> {
    let view = state.settings.registries_view().await?;
    Ok(Json(RegistriesSettings::new(
        view,
        &*state.password_present,
    )))
}

async fn set_registries(
    CurrentAdmin(admin): CurrentAdmin,
    State(state): State<AppState>,
    JsonBody(rows): JsonBody<Vec<RegistryChangeDto>>,
) -> Result<StatusCode, ApiError> {
    let registries = rows.into_iter().map(Registry::from).collect();
    state.settings.set_registries(&admin, registries).await?;
    Ok(StatusCode::NO_CONTENT)
}
