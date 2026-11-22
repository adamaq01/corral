use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::http::header::LOCATION;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use corral_api::{
    ApiError, Identities, LoginKind, ProviderCookies, ProviderDescription, ProviderRoutes,
    QueryParams, SignedIn, provider_path, return_path, sign_in_failed,
};
use serde::Deserialize;

use crate::OidcProvider;
use crate::login_secrets::LoginSecrets;

const LOGIN_COOKIE: &str = "login";
const LOGIN_TTL: Duration = Duration::from_mins(10);

#[derive(Deserialize)]
struct Start {
    next: Option<String>,
}

#[derive(Deserialize)]
struct Callback {
    state: Option<String>,
    code: Option<String>,
}

impl OidcProvider {
    /// `GET /start?next=<path>` sends the browser to the issuer, which sends it back to
    /// `GET /callback`, which sends it on to the path signed in, or to the sign-in page with the
    /// error.
    #[must_use]
    pub fn routes(self: &Arc<Self>) -> ProviderRoutes {
        ProviderRoutes {
            id: Self::ID.to_owned(),
            description: ProviderDescription {
                label: "Single sign-on".to_owned(),
                kind: LoginKind::Redirect,
                login: format!("{}/start", provider_path(Self::ID)),
                accounts: None,
            },
            routes: Router::new()
                .route("/start", get(start_route))
                .route("/callback", get(callback))
                .with_state(self.clone()),
        }
    }
}

async fn start_route(
    State(oidc): State<Arc<OidcProvider>>,
    cookies: ProviderCookies,
    QueryParams(start): QueryParams<Start>,
) -> Response {
    let (url, secrets) = oidc.begin(return_path(start.next.as_deref()));
    let cookies = cookies.set(LOGIN_COOKIE, &secrets, LOGIN_TTL);
    (StatusCode::FOUND, cookies, [(LOCATION, url)]).into_response()
}

/// Ends the login whatever the outcome, so an answer is never checked twice.
async fn callback(
    State(oidc): State<Arc<OidcProvider>>,
    cookies: ProviderCookies,
    identities: Identities,
    QueryParams(answer): QueryParams<Callback>,
) -> Response {
    let secrets = cookies.get::<LoginSecrets>(LOGIN_COOKIE);
    let cookies = cookies.remove(LOGIN_COOKIE);
    match sign_in(&oidc, secrets, answer, identities).await {
        Ok((signed_in, return_to)) => (
            StatusCode::FOUND,
            cookies,
            signed_in,
            [(LOCATION, return_to)],
        )
            .into_response(),
        Err(error) => (
            StatusCode::FOUND,
            cookies,
            [(LOCATION, sign_in_failed(&error))],
        )
            .into_response(),
    }
}

async fn sign_in(
    oidc: &OidcProvider,
    secrets: Option<LoginSecrets>,
    answer: Callback,
    identities: Identities,
) -> Result<(SignedIn, String), ApiError> {
    let secrets = secrets.ok_or_else(|| {
        ApiError::new(StatusCode::BAD_REQUEST, "no login in progress, start again")
    })?;
    let return_to = secrets.return_to.clone();
    let identity = oidc
        .finish(secrets, answer.state.as_deref(), answer.code.as_deref())
        .await?;
    Ok((identities.sign_in(identity).await?, return_to))
}
