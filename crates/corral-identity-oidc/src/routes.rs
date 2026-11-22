use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::http::header::LOCATION;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use corral_api::{
    ApiError, Identities, ProviderCookies, ProviderRoutes, QueryParams, SignedIn, provider_path,
};
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::OidcProvider;
use crate::login_secrets::LoginSecrets;

const LOGIN_COOKIE: &str = "login";
const LOGIN_TTL: Duration = Duration::from_mins(10);

#[derive(Deserialize)]
struct Callback {
    state: Option<String>,
    code: Option<String>,
}

impl OidcProvider {
    /// `GET /start` sends the browser to the issuer, which sends it back to `GET /callback`.
    #[must_use]
    pub fn routes(self: &Arc<Self>) -> ProviderRoutes {
        let start = format!("{}/start", provider_path(Self::ID));
        ProviderRoutes {
            id: Self::ID.to_owned(),
            description: Map::from_iter([
                ("label".to_owned(), Value::from("Single sign-on")),
                ("kind".to_owned(), Value::from("redirect")),
                ("login".to_owned(), Value::from(start)),
            ]),
            routes: Router::new()
                .route("/start", get(start_route))
                .route("/callback", get(callback))
                .with_state(self.clone()),
        }
    }
}

async fn start_route(State(oidc): State<Arc<OidcProvider>>, cookies: ProviderCookies) -> Response {
    let (url, secrets) = oidc.begin();
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
        Ok(signed_in) => (StatusCode::FOUND, cookies, signed_in, [(LOCATION, "/")]).into_response(),
        Err(error) => (cookies, error).into_response(),
    }
}

async fn sign_in(
    oidc: &OidcProvider,
    secrets: Option<LoginSecrets>,
    answer: Callback,
    identities: Identities,
) -> Result<SignedIn, ApiError> {
    let secrets = secrets.ok_or_else(|| {
        ApiError::new(StatusCode::BAD_REQUEST, "no login in progress, start again")
    })?;
    let identity = oidc
        .finish(secrets, answer.state.as_deref(), answer.code.as_deref())
        .await?;
    identities.sign_in(identity).await
}
