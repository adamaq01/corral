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
use corral_core::app::{
    Arrival, Catalog, Choice, Entrance, FailureWindow, Lobby, Made, Network, Offer, SshAccess,
};
use corral_core::domain::{Repository, Sandbox, SandboxId, SshUsername, Target};
use corral_core::ports::ActivitySignals;

use crate::api::{
    self, ARRIVE, AUTHENTICATE, Arrive, Arrived, Authenticate, Authenticated, CHECK_IMAGE,
    CLIENT_STATE, CREATE, CheckImage, ClientState, Create, Created, Enter, Entered, EntryKind,
    ImageChecked, ImageChoice, LOBBY, MadeEntry, NamedImages, OfferedEntry, RepositorySpec,
    SizeChoice, WORKED, Worked,
};
use crate::{GateConfig, GateSecret};

const MAX_FAILED_CONNECTIONS: NonZeroUsize = NonZeroUsize::new(10).unwrap();
const LOCKOUT_WINDOW: Duration = Duration::from_mins(5);

struct Gatekeeper {
    access: Arc<SshAccess>,
    lobby: Arc<Lobby>,
    activity: Arc<dyn ActivitySignals>,
    settings: GateConfig,
    failures: FailureWindow<Network>,
}

/// The server's half of the internal API, which answers only the gate.
pub fn router(
    access: Arc<SshAccess>,
    lobby: Arc<Lobby>,
    activity: Arc<dyn ActivitySignals>,
    settings: GateConfig,
    secret: GateSecret,
) -> Router {
    let gatekeeper = Gatekeeper {
        access,
        lobby,
        activity,
        settings,
        failures: FailureWindow::new(MAX_FAILED_CONNECTIONS, LOCKOUT_WINDOW),
    };
    Router::new()
        .route(AUTHENTICATE, post(authenticate))
        .route(ARRIVE, post(arrive))
        .route(WORKED, post(worked))
        .route(CLIENT_STATE, post(client_state))
        .route(LOBBY, post(enter))
        .route(CHECK_IMAGE, post(check_image))
        .route(CREATE, post(create))
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

async fn enter(
    State(gatekeeper): State<Arc<Gatekeeper>>,
    request: Result<Json<Enter>, JsonRejection>,
) -> Response {
    let Ok(Json(Enter { username })) = request else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let lobby = &gatekeeper.lobby;
    match lobby.enter(&username).await {
        Ok(Entrance::Sandbox) => Json(Entered::Sandbox).into_response(),
        Ok(Entrance::Lobby(offer)) => match lobby.catalog().await {
            Ok(catalog) => Json(Entered::Lobby {
                offer: offered(&catalog, offer),
            })
            .into_response(),
            Err(error) => internal_error(&error),
        },
        Ok(Entrance::Refused(reason)) => Json(Entered::Refused { reason }).into_response(),
        Err(error) => internal_error(&error),
    }
}

async fn check_image(
    State(gatekeeper): State<Arc<Gatekeeper>>,
    request: Result<Json<CheckImage>, JsonRejection>,
) -> Response {
    let Ok(Json(CheckImage { reference })) = request else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    match gatekeeper.lobby.check_image(&reference).await {
        Ok(()) => Json(ImageChecked { problem: None }).into_response(),
        Err(error) => match told(&error) {
            Some(problem) => Json(ImageChecked {
                problem: Some(problem.to_owned()),
            })
            .into_response(),
            None => internal_error(&error),
        },
    }
}

async fn create(
    State(gatekeeper): State<Arc<Gatekeeper>>,
    request: Result<Json<Create>, JsonRejection>,
) -> Response {
    let Ok(Json(Create { username, spec })) = request else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let repository = spec
        .repository
        .map(|repository| Repository::new(&repository.url, repository.branch.as_deref()))
        .transpose();
    let repository = match repository {
        Ok(repository) => repository,
        Err(error) => return refused(&error),
    };
    let choice = Choice {
        image: spec.image,
        size: spec.size,
        setup: spec.setup,
        repository,
    };
    // As in arrive: the sandbox is made whole even when the gate stops waiting.
    let lobby = gatekeeper.lobby.clone();
    let made = tokio::spawn(async move { lobby.create(&username, choice).await })
        .await
        .unwrap_or_else(|failed| Err(Error::internal("create", failed)));
    match made {
        Ok(made) => Json(Created::Made {
            made: made_of(made),
        })
        .into_response(),
        Err(error) => refused(&error),
    }
}

fn refused(error: &Error) -> Response {
    match told(error) {
        Some(reason) => Json(Created::Refused {
            reason: reason.to_owned(),
        })
        .into_response(),
        None => internal_error(error),
    }
}

/// What an error tells the person who caused it, unless it is Corral's own failure.
fn told(error: &Error) -> Option<&str> {
    match error {
        Error::Invalid(why)
        | Error::NotFound(why)
        | Error::Conflict(why)
        | Error::LimitReached(why)
        | Error::Inactive(why) => Some(why),
        Error::Forbidden | Error::Internal { .. } => None,
    }
}

fn offered(catalog: &Catalog, offer: Offer) -> api::Offer {
    api::Offer {
        handle: offer.owner.to_string(),
        name: offer.name.map(|name| name.to_string()),
        images: catalog
            .offered_images()
            .map(|image| ImageChoice {
                key: image.key.to_string(),
                label: image.label.clone(),
            })
            .collect(),
        named_images: catalog.named_images().map(|named| NamedImages {
            registries: named.registries.clone(),
            max_bytes: named.max_bytes.get(),
        }),
        sizes: catalog
            .offered_sizes()
            .map(|size| SizeChoice {
                key: size.key.to_string(),
                label: size.label.clone(),
                cpu_millis: size.resources.cpu_millis.limit(),
                memory_bytes: size.resources.memory_bytes.limit(),
                home_bytes: size.resources.home_bytes.get(),
            })
            .collect(),
        default_image: offer.defaults.image.to_string(),
        default_size: offer.defaults.size.to_string(),
        entries: offer
            .entries
            .iter()
            .map(|view| {
                let entry = &view.entry;
                let (kind, name) = kind_and_name(&entry.target);
                OfferedEntry {
                    id: entry.id.0.to_string(),
                    kind,
                    name,
                    secret: entry.is_secret(),
                    template: entry.template,
                    apply_to_new: entry.apply_to_new,
                    shared: entry.owner.shared(),
                    shadowed: view.shadowed,
                }
            })
            .collect(),
    }
}

fn made_of(made: Made) -> api::Made {
    let sandbox = made.view.sandbox;
    let ssh_username = SshUsername {
        handle: made.view.owner.handle,
        sandbox: Some(sandbox.name.clone()),
    };
    api::Made {
        name: sandbox.name.to_string(),
        ssh_username: ssh_username.to_string(),
        image: sandbox.image.to_string(),
        size: sandbox.size.to_string(),
        setup: made
            .attached
            .iter()
            .map(|entry| {
                let (kind, name) = kind_and_name(&entry.target);
                MadeEntry {
                    id: entry.id.0.to_string(),
                    kind,
                    name,
                }
            })
            .collect(),
        repository: sandbox.repository.map(|repository| RepositorySpec {
            url: repository.url().to_owned(),
            branch: repository.branch().map(str::to_owned),
        }),
    }
}

fn kind_and_name(target: &Target) -> (EntryKind, String) {
    match target {
        Target::Variable(name) => (EntryKind::Variable, name.to_string()),
        Target::File(path) => (EntryKind::File, path.to_string()),
    }
}

fn internal_error(error: &Error) -> Response {
    tracing::error!(%error, "a gate request failed");
    StatusCode::INTERNAL_SERVER_ERROR.into_response()
}
