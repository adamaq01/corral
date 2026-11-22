use std::collections::BTreeMap;

use axum::extract::State;
use axum::http::StatusCode;
use axum::http::header::LOCATION;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use axum_extra::extract::cookie::PrivateCookieJar;
use chrono::Utc;
use corral_core::domain::User;
use corral_core::ports::{Answer, Challenge};

use crate::client_address::ClientAddress;
use crate::dto::{Me, PasswordLogin, Provider};
use crate::error::ApiError;
use crate::extract::{JsonBody, PathParam, QueryParams};
use crate::session::{Session, SessionCodec};
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
    let listed = state
        .providers
        .iter()
        .map(|provider| Provider::from(provider.as_ref()))
        .collect();
    Json(listed)
}

async fn start(
    State(state): State<AppState>,
    PathParam(id): PathParam<String>,
    jar: PrivateCookieJar,
) -> Result<Response, ApiError> {
    let provider = state.provider(&id)?;
    Ok(match provider.challenge().await? {
        Challenge::Password => Json(Provider::from(provider)).into_response(),
        Challenge::Redirect { url, pending } => (
            jar.add(state.sessions.encode_login(provider.id(), &pending)),
            found(url),
        )
            .into_response(),
    })
}

async fn finish_redirect(
    State(state): State<AppState>,
    PathParam(id): PathParam<String>,
    jar: PrivateCookieJar,
    QueryParams(params): QueryParams<BTreeMap<String, String>>,
) -> Response {
    let outcome = finish_redirect_login(&state, &id, &jar, params).await;
    let jar = jar.remove(SessionCodec::login_removal());
    match outcome {
        Ok(user) => (start_session(&state, jar, &user), found("/".to_owned())).into_response(),
        Err(error) => (jar, error).into_response(),
    }
}

async fn finish_redirect_login(
    state: &AppState,
    id: &str,
    jar: &PrivateCookieJar,
    params: BTreeMap<String, String>,
) -> Result<User, ApiError> {
    let provider = state.provider(id)?;
    let pending = SessionCodec::decode_login(jar, provider.id()).ok_or_else(|| {
        ApiError::new(StatusCode::BAD_REQUEST, "no login in progress, start again")
    })?;
    let identity = provider
        .verify(Answer::Redirect { pending, params })
        .await?;
    Ok(state.accounts.login(identity).await?)
}

async fn finish_password(
    State(state): State<AppState>,
    PathParam(id): PathParam<String>,
    ClientAddress(address): ClientAddress,
    jar: PrivateCookieJar,
    JsonBody(form): JsonBody<PasswordLogin>,
) -> Result<impl IntoResponse, ApiError> {
    let provider = state.provider(&id)?;
    let identity = state
        .logins
        .verify(provider, form.username, form.password, address)
        .await?;
    let user = state.accounts.login(identity).await?;
    Ok((start_session(&state, jar, &user), Json(Me::from(&user))))
}

async fn logout(jar: PrivateCookieJar) -> impl IntoResponse {
    (
        jar.remove(SessionCodec::session_removal()),
        StatusCode::NO_CONTENT,
    )
}

fn start_session(state: &AppState, jar: PrivateCookieJar, user: &User) -> PrivateCookieJar {
    jar.add(
        state
            .sessions
            .encode_session(&Session::new(user.id, Utc::now())),
    )
}

fn found(location: String) -> impl IntoResponse {
    (StatusCode::FOUND, [(LOCATION, location)])
}
