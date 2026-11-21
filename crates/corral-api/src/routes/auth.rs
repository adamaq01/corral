use axum::extract::State;
use axum::http::StatusCode;
use axum::http::header::LOCATION;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use axum_extra::extract::cookie::PrivateCookieJar;
use chrono::Utc;
use corral_core::domain::User;
use corral_core::ports::{IdentityProvider, LoginState};

use super::local_accounts;
use crate::client_address::ClientAddress;
use crate::dto::{Callback, LocalLogin, Me, Methods};
use crate::error::ApiError;
use crate::extract::{JsonBody, QueryParams};
use crate::session::{Session, SessionCodec};
use crate::state::AppState;

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/auth/methods", get(methods))
        .route("/auth/login", get(login))
        .route("/auth/callback", get(callback))
        .route("/auth/local", post(local))
        .route("/auth/logout", post(logout))
}

async fn methods(State(state): State<AppState>) -> Json<Methods> {
    Json(Methods {
        oidc: state.identity.is_some(),
        local: state.local.is_some(),
    })
}

async fn login(
    State(state): State<AppState>,
    jar: PrivateCookieJar,
) -> Result<impl IntoResponse, ApiError> {
    let start = oidc(&state)?.begin().await?;
    Ok((
        jar.add(state.sessions.encode_login(&start.state)),
        found(start.url),
    ))
}

async fn callback(
    State(state): State<AppState>,
    jar: PrivateCookieJar,
    QueryParams(callback): QueryParams<Callback>,
) -> impl IntoResponse {
    let outcome = finish_oidc_login(&state, SessionCodec::decode_login(&jar), &callback).await;
    let jar = jar.remove(SessionCodec::login_removal());
    match outcome {
        Ok(user) => (start_session(&state, jar, &user), found("/".to_owned())).into_response(),
        Err(error) => (jar, error).into_response(),
    }
}

async fn finish_oidc_login(
    state: &AppState,
    login: Option<LoginState>,
    callback: &Callback,
) -> Result<User, ApiError> {
    let identity = oidc(state)?;
    let login = login.ok_or_else(|| {
        ApiError::new(StatusCode::BAD_REQUEST, "no login in progress, start again")
    })?;
    let principal = identity
        .complete(login, &callback.code, &callback.state)
        .await?;
    Ok(state.accounts.login(principal).await?)
}

async fn local(
    State(state): State<AppState>,
    ClientAddress(address): ClientAddress,
    jar: PrivateCookieJar,
    JsonBody(form): JsonBody<LocalLogin>,
) -> Result<impl IntoResponse, ApiError> {
    let local = local_accounts(&state)?;
    if !state.logins.admit(&form.username, address) {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "too many login attempts, try again later",
        ));
    }
    let principal = local.authenticate(&form.username, form.password).await?;
    let user = state.accounts.login(principal).await?;
    Ok((start_session(&state, jar, &user), Json(Me::from(&user))))
}

async fn logout(jar: PrivateCookieJar) -> impl IntoResponse {
    (
        jar.remove(SessionCodec::session_removal()),
        StatusCode::NO_CONTENT,
    )
}

fn oidc(state: &AppState) -> Result<&dyn IdentityProvider, ApiError> {
    state.identity.as_deref().ok_or_else(ApiError::not_found)
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
