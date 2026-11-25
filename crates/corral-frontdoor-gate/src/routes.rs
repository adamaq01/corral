use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::rejection::JsonRejection;
use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use corral_core::Error;
use corral_core::app::{Arrival, FailureWindow, Network, SshAccess};
use corral_core::domain::{Sandbox, SandboxId};
use corral_core::ports::ActivitySignals;

use crate::api::{
    ARRIVE, AUTHENTICATE, Arrive, Arrived, Authenticate, Authenticated, CLIENT_STATE, ClientState,
    WORKED, Worked,
};
use crate::{GateConfig, GateSecret};

const MAX_FAILED_CONNECTIONS: NonZeroUsize = NonZeroUsize::new(10).unwrap();
const LOCKOUT_WINDOW: Duration = Duration::from_mins(5);

struct Gatekeeper {
    access: Arc<SshAccess>,
    activity: Arc<dyn ActivitySignals>,
    settings: GateConfig,
    failures: FailureWindow<Network>,
}

/// The server's half of the internal API, which answers only the gate.
pub fn router(
    access: Arc<SshAccess>,
    activity: Arc<dyn ActivitySignals>,
    settings: GateConfig,
    secret: GateSecret,
) -> Router {
    let gatekeeper = Gatekeeper {
        access,
        activity,
        settings,
        failures: FailureWindow::new(MAX_FAILED_CONNECTIONS, LOCKOUT_WINDOW),
    };
    Router::new()
        .route(AUTHENTICATE, post(authenticate))
        .route(ARRIVE, post(arrive))
        .route(WORKED, post(worked))
        .route(CLIENT_STATE, post(client_state))
        .with_state(Arc::new(gatekeeper))
        .route_layer(middleware::from_fn(move |request: Request, next: Next| {
            let admitted = secret.admits(
                request
                    .headers()
                    .get(header::AUTHORIZATION)
                    .and_then(|value| value.to_str().ok()),
            );
            async move {
                if admitted {
                    next.run(request).await
                } else {
                    StatusCode::UNAUTHORIZED.into_response()
                }
            }
        }))
}

async fn authenticate(
    State(gatekeeper): State<Arc<Gatekeeper>>,
    request: Result<Json<Authenticate>, JsonRejection>,
) -> Response {
    let Ok(Json(request)) = request else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    // tokio's clock, so paused-time tests can move past the window.
    let now = tokio::time::Instant::now().into_std();
    let network = gatekeeper
        .settings
        .lockout_network(request.client_address.ip());
    let Some(attempt) =
        gatekeeper
            .failures
            .admit(network.as_slice(), Some(&request.connection), now)
    else {
        return Json(Authenticated { accepted: false }).into_response();
    };
    let access = &gatekeeper.access;
    let checked = if request.proven {
        access
            .authenticate(&request.username, &request.credential)
            .await
    } else {
        access
            .may_present(&request.username, &request.credential)
            .await
    };
    let accepted = match checked {
        Ok(accepted) => accepted,
        Err(error) => {
            attempt.release();
            return internal_error(&error);
        }
    };
    match (accepted, request.proven) {
        (true, true) => attempt.succeed(),
        // An offer that may go on is no failure yet: its proof decides.
        (true, false) => attempt.release(),
        // The attempt stays counted.
        (false, _) => {}
    }
    Json(Authenticated { accepted }).into_response()
}

async fn arrive(
    State(gatekeeper): State<Arc<Gatekeeper>>,
    request: Result<Json<Arrive>, JsonRejection>,
) -> Response {
    let Ok(Json(request)) = request else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    // A gate that stops waiting drops this request. The wake runs on, so the sandbox it creates
    // or wakes is whole for the next connection.
    let access = gatekeeper.access.clone();
    let username = request.username.clone();
    let arrival = tokio::spawn(async move {
        if request.wake {
            access.arrive(&username).await
        } else {
            access.look(&username).await
        }
    })
    .await
    .unwrap_or_else(|failed| Err(Error::internal("arrive", failed)));
    let kept = async |sandbox: &Sandbox| match &request.client {
        Some(client) => gatekeeper.access.client_state(&sandbox.id, client).await,
        None => Ok(None),
    };
    let arrived = match arrival {
        Ok(Arrival::Ready(sandbox)) => kept(&sandbox).await.map(|client_state| Arrived::Ready {
            client_state,
            sandbox: sandbox.id.0,
            name: sandbox.name.to_string(),
        }),
        Ok(Arrival::Asleep(sandbox)) => kept(&sandbox).await.map(|client_state| Arrived::Asleep {
            client_state,
            sandbox: sandbox.id.0,
            name: sandbox.name.to_string(),
        }),
        Ok(Arrival::Waking(sandbox)) => Ok(Arrived::Waking {
            name: sandbox.name.to_string(),
        }),
        Ok(Arrival::Refused(reason)) => Ok(Arrived::Refused { reason }),
        Err(error) => Err(error),
    };
    match arrived {
        Ok(arrived) => Json(arrived).into_response(),
        Err(error) => internal_error(&error),
    }
}

async fn worked(
    State(gatekeeper): State<Arc<Gatekeeper>>,
    request: Result<Json<Worked>, JsonRejection>,
) -> StatusCode {
    let Ok(Json(Worked { sandboxes })) = request else {
        return StatusCode::BAD_REQUEST;
    };
    let sandboxes: Vec<SandboxId> = sandboxes.into_iter().map(SandboxId).collect();
    gatekeeper.activity.worked(&sandboxes);
    StatusCode::NO_CONTENT
}

async fn client_state(
    State(gatekeeper): State<Arc<Gatekeeper>>,
    request: Result<Json<ClientState>, JsonRejection>,
) -> Response {
    let Ok(Json(kept)) = request else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let sandbox = SandboxId(kept.sandbox);
    match gatekeeper
        .access
        .keep_client_state(&sandbox, &kept.client, &kept.state)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => internal_error(&error),
    }
}

fn internal_error(error: &Error) -> Response {
    tracing::error!(%error, "a gate request failed");
    StatusCode::INTERNAL_SERVER_ERROR.into_response()
}
