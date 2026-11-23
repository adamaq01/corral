//! What a sandbox may do to itself, with the token Corral issued it at its start.

use std::future::Future;

use axum::extract::{FromRequestParts, State};
use axum::http::StatusCode;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use corral_core::app::CallingSandbox;

use crate::dto::{ConfirmDeletion, SandboxDto, ScheduledDeletion};
use crate::error::ApiError;
use crate::extract::JsonBody;
use crate::state::{AppState, shared};

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/sandbox", get(show))
        .route("/api/sandbox/restart", post(restart))
        .route("/api/sandbox/stop", post(stop))
        .route("/api/sandbox/delete", post(delete))
}

/// The sandbox whose current token the request carries as `Authorization: Bearer <token>`.
pub(crate) struct CurrentSandbox(pub CallingSandbox);

impl<S: Send + Sync> FromRequestParts<S> for CurrentSandbox {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, ApiError> {
        let api = shared(parts)?;
        let token = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .ok_or_else(refused)?;
        let calling = api.sandboxes.authenticate(token).await?;
        calling.map(Self).ok_or_else(refused)
    }
}

fn refused() -> ApiError {
    ApiError::new(
        StatusCode::UNAUTHORIZED,
        "not the token of a running sandbox",
    )
}

async fn show(
    CurrentSandbox(calling): CurrentSandbox,
    State(state): State<AppState>,
) -> Result<Json<SandboxDto>, ApiError> {
    let view = state.sandboxes.get(calling.owner(), calling.name()).await?;
    Ok(Json(SandboxDto::new(view, &state.ssh)))
}

async fn restart(
    CurrentSandbox(calling): CurrentSandbox,
    State(state): State<AppState>,
) -> StatusCode {
    let sandbox = calling.name().to_string();
    accept("restart", sandbox, async move {
        state
            .sandboxes
            .restart(calling.owner(), calling.name())
            .await
    })
}

async fn stop(
    CurrentSandbox(calling): CurrentSandbox,
    State(state): State<AppState>,
) -> StatusCode {
    let sandbox = calling.name().to_string();
    accept("stop", sandbox, async move {
        state.sandboxes.stop(calling.owner(), calling.name()).await
    })
}

/// Schedules the deletion, then stops the sandbox at once, as an accepted stop.
async fn delete(
    CurrentSandbox(calling): CurrentSandbox,
    State(state): State<AppState>,
    JsonBody(ConfirmDeletion { confirm }): JsonBody<ConfirmDeletion>,
) -> Result<(StatusCode, Json<ScheduledDeletion>), ApiError> {
    let delete_at = state
        .sandboxes
        .schedule_deletion(&calling, &confirm, Utc::now())
        .await?;
    let sandbox = calling.name().to_string();
    let status = accept("stop before a deletion", sandbox, async move {
        state.sandboxes.stop(calling.owner(), calling.name()).await
    });
    Ok((status, Json(ScheduledDeletion { delete_at })))
}

/// Runs `action` on its own: it ends the session that asked for it, whose connection may close
/// before it is done.
fn accept(
    action: &'static str,
    sandbox: String,
    run: impl Future<Output = corral_core::Result<()>> + Send + 'static,
) -> StatusCode {
    tokio::spawn(async move {
        if let Err(error) = run.await {
            tracing::error!(action, sandbox, %error, "a sandbox's request to act on itself failed");
        }
    });
    StatusCode::ACCEPTED
}
