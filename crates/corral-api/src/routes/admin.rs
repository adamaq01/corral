use std::collections::HashMap;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use corral_core::Error;
use corral_core::domain::UserId;
use uuid::Uuid;

use super::local_accounts;
use crate::auth::CurrentUser;
use crate::dto::{LocalAccountDto, NewLocalAccount, NewPassword, OwnedSandbox, UserDto};
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
            "/api/admin/local-accounts",
            get(local_account_list).post(create_local_account),
        )
        .route(
            "/api/admin/local-accounts/{username}",
            delete(delete_local_account),
        )
        .route(
            "/api/admin/local-accounts/{username}/password",
            put(set_password),
        )
}

async fn users(
    CurrentUser(admin): CurrentUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<UserDto>>, ApiError> {
    let users = state.accounts.list(&admin).await?;
    Ok(Json(users.into_iter().map(UserDto::from).collect()))
}

async fn enable(
    CurrentUser(admin): CurrentUser,
    State(state): State<AppState>,
    PathParam(id): PathParam<Uuid>,
) -> Result<Json<UserDto>, ApiError> {
    let user = state.accounts.set_enabled(&admin, UserId(id), true).await?;
    Ok(Json(UserDto::from(user)))
}

async fn disable(
    CurrentUser(admin): CurrentUser,
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
    CurrentUser(admin): CurrentUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<OwnedSandbox>>, ApiError> {
    let views = state.sandboxes.list_all(&admin).await?;
    let handles: HashMap<_, _> = state
        .accounts
        .list(&admin)
        .await?
        .into_iter()
        .map(|user| (user.id, user.handle))
        .collect();
    let owned = views
        .into_iter()
        .map(|view| {
            let owner = handles.get(&view.sandbox.owner).ok_or_else(|| {
                Error::internal(
                    "store",
                    format!("sandbox {} has no owner", view.sandbox.id.0),
                )
            })?;
            Ok(OwnedSandbox::new(view, owner, &state.catalog.ssh))
        })
        .collect::<Result<_, Error>>()?;
    Ok(Json(owned))
}

async fn local_account_list(
    CurrentUser(admin): CurrentUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<LocalAccountDto>>, ApiError> {
    let accounts = local_accounts(&state)?.list(&admin).await?;
    Ok(Json(
        accounts.into_iter().map(LocalAccountDto::from).collect(),
    ))
}

async fn create_local_account(
    CurrentUser(admin): CurrentUser,
    State(state): State<AppState>,
    JsonBody(account): JsonBody<NewLocalAccount>,
) -> Result<StatusCode, ApiError> {
    local_accounts(&state)?
        .create(
            Some(&admin),
            &account.username,
            account.password,
            account.role,
        )
        .await?;
    Ok(StatusCode::CREATED)
}

async fn set_password(
    CurrentUser(admin): CurrentUser,
    State(state): State<AppState>,
    PathParam(username): PathParam<String>,
    JsonBody(body): JsonBody<NewPassword>,
) -> Result<StatusCode, ApiError> {
    local_accounts(&state)?
        .set_password(&admin, &username, body.password)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_local_account(
    CurrentUser(admin): CurrentUser,
    State(state): State<AppState>,
    PathParam(username): PathParam<String>,
) -> Result<StatusCode, ApiError> {
    local_accounts(&state)?.delete(&admin, &username).await?;
    Ok(StatusCode::NO_CONTENT)
}
