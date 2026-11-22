use std::collections::BTreeMap;

use axum::extract::State;
use axum::http::StatusCode;
use axum::http::header::LOCATION;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use corral_core::domain::User;
use corral_core::ports::{Answer, Challenge};

use crate::client_address::ClientAddress;
use crate::dto::{Me, PasswordLogin, Provider};
use crate::error::ApiError;
use crate::extract::{JsonBody, PathParam, QueryParams};
use crate::session::Session;
use crate::state::AppState;

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/auth/providers", get(providers))
        .route("/auth/{provider}/start", get(start))
        .route(
            "/auth/{provider}/finish",
            get(finish_redirect).post(finish_password),
        )
        .route("/auth/logout", post(logout))
}

async fn providers(State(state): State<AppState>) -> Json<Vec<Provider>> {
    Json(state.providers.iter().map(Provider::from).collect())
}

async fn start(
    State(state): State<AppState>,
    PathParam(id): PathParam<String>,
    session: Session,
) -> Result<Response, ApiError> {
    let provider = state.providers.get(&id)?;
    Ok(match provider.challenge().await? {
        Challenge::Password => Json(Provider::from(provider)).into_response(),
        Challenge::Redirect { url, pending } => {
            (session.start_login(provider.id(), &pending), found(url)).into_response()
        }
    })
}

async fn finish_redirect(
    State(state): State<AppState>,
    PathParam(id): PathParam<String>,
    session: Session,
    QueryParams(params): QueryParams<BTreeMap<String, String>>,
) -> Response {
    let outcome = finish_redirect_login(&state, &id, &session, params).await;
    let session = session.end_login();
    match outcome {
        Ok(user) => (session.start(user.id, Utc::now()), found("/".to_owned())).into_response(),
        Err(error) => (session, error).into_response(),
    }
}

async fn finish_redirect_login(
    state: &AppState,
    id: &str,
    session: &Session,
    params: BTreeMap<String, String>,
) -> Result<User, ApiError> {
    let provider = state.providers.get(id)?;
    let pending = session.pending_login(provider.id()).ok_or_else(|| {
        ApiError::new(StatusCode::BAD_REQUEST, "no login in progress, start again")
    })?;
    let identity = provider
        .verify(Answer::Redirect { pending, params })
        .await?;
    Ok(state.accounts.login(provider.id(), identity).await?)
}

async fn finish_password(
    State(state): State<AppState>,
    PathParam(id): PathParam<String>,
    ClientAddress(address): ClientAddress,
    session: Session,
    JsonBody(form): JsonBody<PasswordLogin>,
) -> Result<impl IntoResponse, ApiError> {
    let identity = state
        .providers
        .verify_password(&id, form.username, form.password, address)
        .await?;
    let user = state.accounts.login(&id, identity).await?;
    Ok((session.start(user.id, Utc::now()), Json(Me::from(&user))))
}

async fn logout(session: Session) -> impl IntoResponse {
    (session.end(), StatusCode::NO_CONTENT)
}

fn found(location: String) -> impl IntoResponse {
    (StatusCode::FOUND, [(LOCATION, location)])
}
