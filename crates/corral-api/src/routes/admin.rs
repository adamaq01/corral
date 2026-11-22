use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use corral_core::domain::UserId;
use uuid::Uuid;

use crate::auth::CurrentAdmin;
use crate::dto::{ManagedAccountDto, NewAccount, NewPassword, OwnedSandbox, UserDto};
use crate::error::ApiError;
use crate::extract::{JsonBody, PathParam};
use crate::state::AppState;

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/admin/users", get(users))
        .route("/api/admin/users/{id}/enable", post(enable))
        .route("/api/admin/users/{id}/disable", post(disable))
        .route("/api/admin/sandboxes", get(sandboxes))
        .route(
            "/api/admin/providers/{provider}/accounts",
            get(accounts).post(create_account),
        )
        .route(
            "/api/admin/providers/{provider}/accounts/{subject}",
            delete(delete_account),
        )
        .route(
            "/api/admin/providers/{provider}/accounts/{subject}/password",
            put(set_password),
        )
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

async fn accounts(
    CurrentAdmin(admin): CurrentAdmin,
    State(state): State<AppState>,
    PathParam(provider): PathParam<String>,
) -> Result<Json<Vec<ManagedAccountDto>>, ApiError> {
    let accounts = state.accounts.managed_accounts(&admin, &provider).await?;
    Ok(Json(
        accounts.into_iter().map(ManagedAccountDto::from).collect(),
    ))
}

async fn create_account(
    CurrentAdmin(admin): CurrentAdmin,
    State(state): State<AppState>,
    PathParam(provider): PathParam<String>,
    JsonBody(account): JsonBody<NewAccount>,
) -> Result<StatusCode, ApiError> {
    state
        .accounts
        .create_account(
            &admin,
            &provider,
            &account.username,
            account.password,
            account.role,
        )
        .await?;
    Ok(StatusCode::CREATED)
}

async fn set_password(
    CurrentAdmin(admin): CurrentAdmin,
    State(state): State<AppState>,
    PathParam((provider, subject)): PathParam<(String, String)>,
    JsonBody(body): JsonBody<NewPassword>,
) -> Result<StatusCode, ApiError> {
    state
        .accounts
        .set_password(&admin, &provider, &subject, body.password)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_account(
    CurrentAdmin(admin): CurrentAdmin,
    State(state): State<AppState>,
    PathParam((provider, subject)): PathParam<(String, String)>,
) -> Result<StatusCode, ApiError> {
    state
        .accounts
        .delete_account(&admin, &provider, &subject)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
