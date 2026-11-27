use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::http::header::LOCATION;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use corral_api::{
    ApiError, CurrentUser, Identities, LoginKind, ProviderCookies, ProviderDescription,
    ProviderRoutes, QueryParams, SignedIn, TypeScript, provider_path, return_path, sign_in_failed,
};
use corral_core::ports::Identity;
use serde::Deserialize;

use crate::login_secrets::LoginSecrets;
use crate::{OidcProvider, OpksshLogin};

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
    /// error. With opkssh on, `GET /opkssh` tells this provider's signed-in users how to log in.
    #[must_use]
    pub fn routes(self: &Arc<Self>) -> ProviderRoutes {
        let path = provider_path(Self::ID);
        let mut routes = Router::new()
            .route("/start", get(start_route))
            .route("/callback", get(callback));
        if self.opkssh_login().is_some() {
            routes = routes.route("/opkssh", get(opkssh));
        }
        ProviderRoutes {
            id: Self::ID.to_owned(),
            description: ProviderDescription {
                label: "Single sign-on".to_owned(),
                kind: LoginKind::Redirect,
                login: format!("{path}/start"),
                accounts: None,
                opkssh: self.opkssh_login().map(|_| format!("{path}/opkssh")),
            },
            routes: routes.with_state(self.clone()),
        }
    }

    /// The types its routes answer with, for the web UI.
    pub fn typescript(types: &mut TypeScript) {
        types.add::<OpksshLogin>();
    }
}

/// 404 for users who signed in elsewhere, whose certificates this provider cannot vouch for.
async fn opkssh(
    State(oidc): State<Arc<OidcProvider>>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<OpksshLogin>, ApiError> {
    match (oidc.opkssh_login(), user.subject_at(OidcProvider::ID)) {
        (Some(login), Some(_)) => Ok(Json(login.clone())),
        _ => Err(ApiError::not_found()),
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
    let (identity, return_to) = finish(oidc, secrets, answer).await.inspect_err(|error| {
        let (provider, reason) = (OidcProvider::ID, error.to_string());
        tracing::warn!(%provider, reason, "a sign-in failed");
    })?;
    Ok((identities.sign_in(identity).await?, return_to))
}

/// The identity the provider's answer establishes, and where its login returns to.
async fn finish(
    oidc: &OidcProvider,
    secrets: Option<LoginSecrets>,
    answer: Callback,
) -> Result<(Identity, String), ApiError> {
    let secrets = secrets.ok_or_else(|| {
        ApiError::new(StatusCode::BAD_REQUEST, "no login in progress, start again")
    })?;
    let return_to = secrets.return_to.clone();
    let identity = oidc
        .finish(secrets, answer.state.as_deref(), answer.code.as_deref())
        .await?;
    Ok((identity, return_to))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Duration;

    use axum::Router;
    use axum::http::StatusCode;
    use corral_api::router;
    use corral_api::testing::{Browser, api, trusting_provider};
    use corral_core::fakes::World;
    use secrecy::SecretString;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::opkssh::Opkssh;
    use crate::{ExpiryPolicy, OidcConfig, OidcProvider, OpksshConfig};

    async fn provider(issuer: &MockServer) -> OidcProvider {
        let base = issuer.uri();
        Mock::given(method("GET"))
            .and(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "issuer": base,
                "authorization_endpoint": format!("{base}/authorize"),
                "token_endpoint": format!("{base}/token"),
                "jwks_uri": format!("{base}/jwks"),
                "response_types_supported": ["code"],
                "subject_types_supported": ["public"],
                "id_token_signing_alg_values_supported": ["RS256"],
            })))
            .mount(issuer)
            .await;
        Mock::given(method("GET"))
            .and(path("/jwks"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"keys": []})))
            .mount(issuer)
            .await;
        let config = OidcConfig {
            issuer: base,
            client_id: "corral".to_owned(),
            scopes: vec!["openid".to_owned()],
            member_group: "corral-users".to_owned(),
            admin_group: "corral-admins".to_owned(),
            opkssh: None,
        };
        OidcProvider::discover(
            config,
            &SecretString::from("secret"),
            "https://corral.example",
        )
        .await
        .unwrap()
    }

    fn with_opkssh(provider: OidcProvider, issuer: &str) -> OidcProvider {
        let config = OpksshConfig {
            client_id: "opkssh".to_owned(),
            policy: ExpiryPolicy::Hours12,
        };
        let opkssh = Opkssh::new(PathBuf::new(), Duration::ZERO, issuer, config).unwrap();
        provider.with_opkssh(opkssh)
    }

    /// Corral with `provider`, whose users sign in at its `/login`, and a provider `local`.
    fn corral(provider: OidcProvider) -> Router {
        let provider = Arc::new(provider);
        let world = World::with_providers(vec![provider.clone()]);
        let mut routes = provider.routes();
        routes.routes = routes
            .routes
            .merge(trusting_provider(OidcProvider::ID).routes);
        router(api(&world), vec![routes, trusting_provider("local")])
    }

    async fn sign_in(browser: &mut Browser, provider: &str, subject: &str) {
        let body = json!({"subject": subject, "username": subject, "role": "member"});
        let reply = browser.post(&format!("/auth/{provider}/login"), body).await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    }

    #[tokio::test]
    async fn only_this_providers_signed_in_users_learn_how_to_log_in_with_opkssh() {
        let issuer = MockServer::start().await;
        let app = corral(with_opkssh(provider(&issuer).await, &issuer.uri()));
        let listed = Browser::new(&app, "1.2.3.4:1")
            .get("/auth/providers")
            .await
            .json();
        assert_eq!(listed[0]["opkssh"], "/auth/oidc/opkssh");

        let mut anonymous = Browser::new(&app, "1.2.3.4:1");
        assert_eq!(
            anonymous.get("/auth/oidc/opkssh").await.status,
            StatusCode::UNAUTHORIZED
        );

        let mut oidc_user = Browser::new(&app, "1.2.3.4:1");
        sign_in(&mut oidc_user, "oidc", "adam").await;
        assert_eq!(
            oidc_user.get("/auth/oidc/opkssh").await.json(),
            json!({"issuer": issuer.uri(), "clientId": "opkssh", "lifetimeHours": 12})
        );

        let mut local_user = Browser::new(&app, "1.2.3.4:1");
        sign_in(&mut local_user, "local", "eve").await;
        assert_eq!(
            local_user.get("/auth/oidc/opkssh").await.status,
            StatusCode::NOT_FOUND
        );
    }

    #[tokio::test]
    async fn without_opkssh_nothing_is_offered() {
        let issuer = MockServer::start().await;
        let app = corral(provider(&issuer).await);
        let listed = Browser::new(&app, "1.2.3.4:1")
            .get("/auth/providers")
            .await
            .json();
        assert_eq!(listed[0]["opkssh"], serde_json::Value::Null);
        let mut oidc_user = Browser::new(&app, "1.2.3.4:1");
        sign_in(&mut oidc_user, "oidc", "adam").await;
        assert_eq!(
            oidc_user.get("/auth/oidc/opkssh").await.status,
            StatusCode::NOT_FOUND
        );
    }
}
