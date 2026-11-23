mod config;

use std::net::SocketAddr;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use corral_core::Error;
use corral_core::app::{FailureWindow, Network, SshAccess};
use corral_core::ports::{Readiness, SshTarget};
use serde::Deserialize;
use serde_json::{Value, json};

pub use config::{Cidr, ContainerSshConfig};

const MAX_FAILED_CONNECTIONS: NonZeroUsize = NonZeroUsize::new(10).unwrap();
const LOCKOUT_WINDOW: Duration = Duration::from_mins(5);

struct FrontDoor {
    access: Arc<SshAccess>,
    settings: ContainerSshConfig,
    failures: FailureWindow<Network>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PubkeyRequest {
    username: String,
    remote_address: String,
    connection_id: String,
    public_key: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConfigRequest {
    authenticated_username: String,
}

pub fn router(access: Arc<SshAccess>, settings: ContainerSshConfig) -> Router {
    let front_door = FrontDoor {
        access,
        settings,
        failures: FailureWindow::new(MAX_FAILED_CONNECTIONS, LOCKOUT_WINDOW),
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
    // tokio's clock, so paused-time tests can move past the window.
    let now = tokio::time::Instant::now().into_std();
    let network = front_door.settings.lockout_network(source.ip());
    let attempt = front_door
        .failures
        .admit(network.as_slice(), Some(&request.connection_id), now);
    let accepted = match attempt {
        None => false,
        Some(attempt) => match front_door
            .access
            .authenticate(&request.username, &request.public_key)
            .await
        {
            Ok(accepted) => {
                if accepted {
                    attempt.succeed();
                }
                accepted
            }
            Err(error) => {
                attempt.release();
                return internal_error(&error);
            }
        },
    };
    Json(json!({"success": accepted, "authenticatedUsername": request.username})).into_response()
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
        Ok(Readiness::Ready(target)) => Json(json!({"config": backend(&target)})).into_response(),
        Ok(Readiness::Waking) => failure(StatusCode::SERVICE_UNAVAILABLE),
        Err(Error::Forbidden) => failure(StatusCode::FORBIDDEN),
        Err(error) => internal_error(&error),
    }
}

fn backend(target: &SshTarget) -> Value {
    match target {
        // Without persistent mode ContainerSSH reads metadata.name as the pod to create. Its
        // default shell is a plain /bin/bash, which reads no profile, unlike an sshd login.
        SshTarget::KubernetesPod { namespace, pod } => json!({
            "backend": "kubernetes",
            "kubernetes": {"pod": {
                "metadata": {"name": pod, "namespace": namespace},
                "mode": "persistent",
                "shellCommand": ["/bin/bash", "-l"],
            }},
        }),
        // ContainerSSH merges this over its own config, which holds the privateKey.
        SshTarget::SshServer {
            host,
            port,
            username,
            host_key_fingerprint,
        } => json!({
            "backend": "sshproxy",
            "sshproxy": {
                "server": host,
                "port": port,
                "username": username,
                "allowedHostKeyFingerprints": [host_key_fingerprint],
            },
        }),
    }
}

// ContainerSSH decodes every body strictly before it checks the status. `{}` decodes, so its
// log names the status instead of a decode error.
fn failure(status: StatusCode) -> Response {
    (status, Json(json!({}))).into_response()
}

fn internal_error(error: &Error) -> Response {
    tracing::error!(%error, "containerssh webhook failed");
    failure(StatusCode::INTERNAL_SERVER_ERROR)
}
