use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use corral_core::Error;
use corral_core::app::SshAccess;
use corral_core::ports::Readiness;
use serde::Serialize;

use crate::FrontDoorConfig;
use crate::dto::{ConfigRequest, ConfigResponse, PubkeyRequest, PubkeyResponse};
use crate::lockout::Lockout;

struct FrontDoor {
    access: Arc<SshAccess>,
    lockout: Lockout,
}

pub fn router(access: Arc<SshAccess>, settings: FrontDoorConfig) -> Router {
    let front_door = FrontDoor {
        access,
        lockout: Lockout::new(settings.lockout),
    };
    Router::new()
        .route("/pubkey", post(pubkey))
        .route("/config", post(config))
        .fallback(|| async { failure(StatusCode::NOT_FOUND) })
        .method_not_allowed_fallback(|| async { failure(StatusCode::METHOD_NOT_ALLOWED) })
        .with_state(Arc::new(front_door))
}

async fn pubkey(
    State(front_door): State<Arc<FrontDoor>>,
    request: Result<Json<PubkeyRequest>, JsonRejection>,
) -> Response {
    let Ok(Json(request)) = request else {
        return failure(StatusCode::BAD_REQUEST);
    };
    let Ok(source) = request.remote_address.parse::<SocketAddr>() else {
        return failure(StatusCode::BAD_REQUEST);
    };
    let address = source.ip();
    if front_door.lockout.is_locked(address) {
        return answer(&PubkeyResponse::new(false, &request.username));
    }
    match front_door
        .access
        .authenticate(&request.username, &request.public_key)
        .await
    {
        Ok(accepted) => {
            if !accepted {
                front_door
                    .lockout
                    .record_failure(address, &request.connection_id);
            }
            answer(&PubkeyResponse::new(accepted, &request.username))
        }
        Err(error) => internal_error(&error),
    }
}

async fn config(
    State(front_door): State<Arc<FrontDoor>>,
    request: Result<Json<ConfigRequest>, JsonRejection>,
) -> Response {
    let Ok(Json(request)) = request else {
        return failure(StatusCode::BAD_REQUEST);
    };
    match front_door
        .access
        .route(&request.authenticated_username)
        .await
    {
        Ok(Readiness::Ready(target)) => answer(&ConfigResponse::for_target(&target)),
        Ok(Readiness::Waking) => failure(StatusCode::SERVICE_UNAVAILABLE),
        Err(Error::Forbidden) => failure(StatusCode::FORBIDDEN),
        Err(error) => internal_error(&error),
    }
}

fn answer(body: &impl Serialize) -> Response {
    Json(body).into_response()
}

#[derive(Serialize)]
struct Empty {}

// ContainerSSH decodes every body strictly before it checks the status. `{}` decodes, so its
// log names the status instead of a decode error.
fn failure(status: StatusCode) -> Response {
    (status, Json(Empty {})).into_response()
}

fn internal_error(error: &Error) -> Response {
    tracing::error!(%error, "containerssh webhook failed");
    failure(StatusCode::INTERNAL_SERVER_ERROR)
}
