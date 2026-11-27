use std::collections::HashMap;
use std::sync::Arc;

use axum::Router;
use axum::http::{StatusCode, header};
use chrono::{Duration, Utc};
use corral_api::router;
use corral_api::testing::{Browser, Reply, api};
use corral_core::fakes::{Logs, World};
use corral_identity_oidc::{OidcConfig, OidcProvider};
use openidconnect::core::{
    CoreGenderClaim, CoreJsonWebKeySet, CoreJweContentEncryptionAlgorithm, CoreJwsSigningAlgorithm,
    CoreProviderMetadata, CoreResponseType, CoreRsaPrivateSigningKey, CoreSubjectIdentifierType,
};
use openidconnect::url::{Url, form_urlencoded};
use openidconnect::{
    AdditionalClaims, Audience, AuthUrl, EmptyAdditionalProviderMetadata, EndUserEmail,
    EndUserPictureUrl, EndUserUsername, IdToken, IdTokenClaims, IssuerUrl, JsonWebKeyId,
    JsonWebKeySetUrl, Nonce, PkceCodeChallenge, PkceCodeVerifier, PrivateSigningKey, ResponseTypes,
    StandardClaims, SubjectIdentifier, TokenUrl,
};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wiremock::matchers::{basic_auth, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const CLIENT_ID: &str = "corral";
const CLIENT_SECRET: &str = "corral-secret";
const BASE_URL: &str = "https://corral.example";
const REDIRECT_URL: &str = "https://corral.example/auth/oidc/callback";
const MEMBERS: &str = "corral-users";
const ADMINS: &str = "corral-admins";
const CODE: &str = "the-code";
const FIRST_KEY: &str = include_str!("fixtures/issuer-key-1.pem");
const ROTATED_KEY: &str = include_str!("fixtures/issuer-key-2.pem");

#[derive(Debug, Deserialize, Serialize)]
struct Groups {
    #[serde(skip_serializing_if = "Option::is_none")]
    groups: Option<Vec<String>>,
}

impl AdditionalClaims for Groups {}

struct Issuer {
    server: MockServer,
    key: CoreRsaPrivateSigningKey,
    picture: Option<&'static str>,
}

impl Issuer {
    async fn start() -> Self {
        let issuer = Self {
            server: MockServer::start().await,
            key: signing_key(FIRST_KEY, "first"),
            picture: None,
        };
        issuer.serve_discovery().await;
        issuer
    }

    async fn rotate_key(&mut self) {
        self.key = signing_key(ROTATED_KEY, "rotated");
        self.server.reset().await;
        self.serve_discovery().await;
    }

    async fn serve_discovery(&self) {
        let base = self.server.uri();
        let metadata = CoreProviderMetadata::new(
            IssuerUrl::new(base.clone()).unwrap(),
            AuthUrl::new(format!("{base}/authorize")).unwrap(),
            JsonWebKeySetUrl::new(format!("{base}/jwks")).unwrap(),
            vec![ResponseTypes::new(vec![CoreResponseType::Code])],
            vec![CoreSubjectIdentifierType::Public],
            vec![CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256],
            EmptyAdditionalProviderMetadata {},
        )
        .set_token_endpoint(Some(TokenUrl::new(format!("{base}/token")).unwrap()));
        let jwks = CoreJsonWebKeySet::new(vec![self.key.as_verification_key()]);
        Mock::given(method("GET"))
            .and(path("/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(metadata))
            .mount(&self.server)
            .await;
        Mock::given(method("GET"))
            .and(path("/jwks"))
            .respond_with(ResponseTemplate::new(200).set_body_json(jwks))
            .mount(&self.server)
            .await;
    }

    async fn corral(&self) -> Router {
        let provider = Arc::new(self.provider().await);
        let world = World::with_providers(vec![provider.clone()]);
        router(api(&world), vec![provider.routes()])
    }

    async fn provider(&self) -> OidcProvider {
        OidcProvider::discover(
            OidcConfig {
                issuer: self.server.uri(),
                client_id: CLIENT_ID.to_owned(),
                scopes: ["openid", "profile", "email", "groups"]
                    .map(String::from)
                    .into(),
                member_group: MEMBERS.to_owned(),
                admin_group: ADMINS.to_owned(),
                opkssh: None,
            },
            &SecretString::from(CLIENT_SECRET),
            BASE_URL,
        )
        .await
        .unwrap()
    }

    fn id_token(&self, nonce: &str, groups: Option<&[&str]>) -> String {
        let claims = IdTokenClaims::<Groups, CoreGenderClaim>::new(
            IssuerUrl::new(self.server.uri()).unwrap(),
            vec![Audience::new(CLIENT_ID.to_owned())],
            Utc::now() + Duration::minutes(5),
            Utc::now(),
            StandardClaims::new(SubjectIdentifier::new("sub-123".to_owned()))
                .set_preferred_username(Some(EndUserUsername::new("Adam".to_owned())))
                .set_email(Some(EndUserEmail::new("adam@example.com".to_owned())))
                .set_picture(
                    self.picture
                        .map(|picture| EndUserPictureUrl::new(picture.to_owned()).into()),
                ),
            Groups {
                groups: groups.map(|groups| groups.iter().map(ToString::to_string).collect()),
            },
        )
        .set_nonce(Some(Nonce::new(nonce.to_owned())));
        IdToken::<Groups, CoreGenderClaim, CoreJweContentEncryptionAlgorithm, CoreJwsSigningAlgorithm>::new(
            claims,
            &self.key,
            CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256,
            None,
            None,
        )
        .unwrap()
        .to_string()
    }

    async fn issue(&self, id_token: String) {
        self.answer_token_request(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "access-123",
            "token_type": "Bearer",
            "expires_in": 300,
            "id_token": id_token,
        })))
        .await;
    }

    async fn answer_token_request(&self, response: ResponseTemplate) {
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(basic_auth(CLIENT_ID, CLIENT_SECRET))
            .respond_with(response)
            .mount(&self.server)
            .await;
    }

    async fn token_requests(&self) -> Vec<HashMap<String, String>> {
        self.server
            .received_requests()
            .await
            .unwrap()
            .into_iter()
            .filter(|request| request.url.path() == "/token")
            .map(|request| form_urlencoded::parse(&request.body).into_owned().collect())
            .collect()
    }
}

fn signing_key(pem: &str, id: &str) -> CoreRsaPrivateSigningKey {
    CoreRsaPrivateSigningKey::from_pem(pem, Some(JsonWebKeyId::new(id.to_owned()))).unwrap()
}

fn param(url: &Url, name: &str) -> String {
    url.query_pairs()
        .find(|(key, _)| key == name)
        .unwrap_or_else(|| panic!("{name} missing from {url}"))
        .1
        .into_owned()
}

fn browser(app: &Router) -> Browser {
    Browser::new(app, "192.0.2.10:50000")
}

/// Starts a login, answering where the issuer was asked to send the browser.
async fn start(browser: &mut Browser) -> Url {
    let start = browser.get("/auth/oidc/start").await;
    assert_eq!(start.status, StatusCode::FOUND, "{}", start.text());
    Url::parse(start.headers[header::LOCATION].to_str().unwrap()).unwrap()
}

async fn callback(browser: &mut Browser, state: &str) -> Reply {
    browser
        .get(&format!("/auth/oidc/callback?code={CODE}&state={state}"))
        .await
}

/// Where a failed login sent the browser.
fn sent_back_to(reply: &Reply) -> &str {
    assert_eq!(reply.status, StatusCode::FOUND, "{}", reply.text());
    reply.headers[header::LOCATION].to_str().unwrap()
}

async fn me(browser: &mut Browser) -> Value {
    browser.get("/api/me").await.json()
}

#[tokio::test]
async fn a_login_round_trip_starts_a_session() {
    let issuer = Issuer::start().await;
    let app = issuer.corral().await;
    let mut browser = browser(&app);
    assert_eq!(
        browser.get("/auth/providers").await.json(),
        json!([{
            "id": "oidc",
            "label": "Single sign-on",
            "kind": "redirect",
            "login": "/auth/oidc/start",
            "accounts": null,
            "opkssh": null,
        }])
    );
    let start = browser.get("/auth/oidc/start").await;
    let login = start.set_cookie("corral_oidc_login");
    assert_eq!(login.path(), Some("/auth/oidc"));
    assert_eq!(login.max_age().unwrap().whole_seconds(), 600);
    let auth = Url::parse(start.headers[header::LOCATION].to_str().unwrap()).unwrap();
    for secret in [param(&auth, "state"), param(&auth, "nonce")] {
        assert!(!login.value().contains(&secret), "{login}");
    }
    issuer
        .issue(issuer.id_token(&param(&auth, "nonce"), Some(&[MEMBERS])))
        .await;

    let done = callback(&mut browser, &param(&auth, "state")).await;

    assert_eq!(done.status, StatusCode::FOUND, "{}", done.text());
    assert_eq!(done.headers[header::LOCATION], "/");
    assert!(!browser.cookies.contains_key("corral_oidc_login"));
    assert_eq!(
        me(&mut browser).await,
        json!({
            "handle": "adam",
            "email": "adam@example.com",
            "picture": null,
            "role": "member",
            "provider": "oidc",
        })
    );
    assert_eq!(auth.path(), "/authorize");
    assert_eq!(param(&auth, "response_type"), "code");
    assert_eq!(param(&auth, "client_id"), CLIENT_ID);
    assert_eq!(param(&auth, "redirect_uri"), REDIRECT_URL);
    assert_eq!(param(&auth, "scope"), "openid profile email groups");
    assert_eq!(param(&auth, "code_challenge_method"), "S256");
    let token_requests = issuer.token_requests().await;
    let [form] = token_requests.as_slice() else {
        panic!("expected one token request, got {token_requests:?}");
    };
    assert_eq!(form["grant_type"], "authorization_code");
    assert_eq!(form["code"], CODE);
    assert_eq!(form["redirect_uri"], REDIRECT_URL);
    let verifier = PkceCodeVerifier::new(form["code_verifier"].clone());
    assert_eq!(
        PkceCodeChallenge::from_code_verifier_sha256(&verifier).as_str(),
        param(&auth, "code_challenge")
    );

    let replay = callback(&mut browser, &param(&auth, "state")).await;
    assert_eq!(
        sent_back_to(&replay),
        "/sign-in?error=no+login+in+progress%2C+start+again"
    );
    assert_eq!(issuer.token_requests().await.len(), 1);
}

#[tokio::test]
async fn a_different_or_missing_returned_state_is_forbidden_before_the_code_is_redeemed() {
    let issuer = Issuer::start().await;
    let app = issuer.corral().await;
    issuer
        .issue(issuer.id_token("any-nonce", Some(&[MEMBERS])))
        .await;
    let mut browser = browser(&app);
    let logs = Logs::capture();

    start(&mut browser).await;
    let forged = callback(&mut browser, "forged-state").await;
    assert!(!browser.cookies.contains_key("corral_oidc_login"));
    start(&mut browser).await;
    let missing = browser
        .get(&format!("/auth/oidc/callback?code={CODE}"))
        .await;

    for reply in [forged, missing] {
        assert_eq!(sent_back_to(&reply), "/sign-in?error=forbidden");
    }
    let refused = logs.of("a sign-in failed");
    assert_eq!(refused.len(), 2, "{:#?}", logs.lines());
    for line in refused {
        assert!(line.starts_with(" WARN"), "{line}");
        assert!(
            line.ends_with("a sign-in failed provider=oidc reason=\"forbidden\""),
            "{line}"
        );
    }
    assert_eq!(issuer.token_requests().await.len(), 0);
    assert_eq!(
        browser.get("/api/me").await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn a_callback_without_our_login_cookie_is_refused() {
    let issuer = Issuer::start().await;
    let app = issuer.corral().await;
    let mut started = browser(&app);
    let auth = start(&mut started).await;
    let state = param(&auth, "state");

    let mut stranger = browser(&app);
    let mut tamperer = browser(&app);
    let mut cookie = started.cookies["corral_oidc_login"].clone();
    cookie.replace_range(..1, if cookie.starts_with('A') { "B" } else { "A" });
    tamperer
        .cookies
        .insert("corral_oidc_login".to_owned(), cookie);
    for browser in [&mut stranger, &mut tamperer] {
        let reply = callback(browser, &state).await;
        assert_eq!(
            sent_back_to(&reply),
            "/sign-in?error=no+login+in+progress%2C+start+again"
        );
    }
    assert_eq!(issuer.token_requests().await.len(), 0);
}

#[tokio::test]
async fn a_callback_without_a_code_is_invalid() {
    let issuer = Issuer::start().await;
    let app = issuer.corral().await;
    let mut browser = browser(&app);
    let auth = start(&mut browser).await;
    let reply = browser
        .get(&format!(
            "/auth/oidc/callback?state={}",
            param(&auth, "state")
        ))
        .await;
    assert_eq!(
        sent_back_to(&reply),
        "/sign-in?error=invalid%3A+the+identity+provider+returned+no+code"
    );
    assert_eq!(issuer.token_requests().await.len(), 0);
}

#[tokio::test]
async fn an_id_token_for_another_nonce_is_forbidden() {
    let issuer = Issuer::start().await;
    let app = issuer.corral().await;
    let mut browser = browser(&app);
    let auth = start(&mut browser).await;
    issuer
        .issue(issuer.id_token("another-nonce", Some(&[MEMBERS])))
        .await;

    let reply = callback(&mut browser, &param(&auth, "state")).await;

    assert_eq!(sent_back_to(&reply), "/sign-in?error=forbidden");
}

#[tokio::test]
async fn groups_decide_the_role() {
    let issuer = Issuer::start().await;
    let app = issuer.corral().await;
    for (groups, role) in [
        (Some(&[MEMBERS, ADMINS][..]), Some("admin")),
        (Some(&[ADMINS][..]), Some("admin")),
        (Some(&["other", MEMBERS][..]), Some("member")),
        (Some(&["other"][..]), None),
        (Some(&[][..]), None),
        (None, None),
    ] {
        issuer.server.reset().await;
        issuer.serve_discovery().await;
        let mut browser = browser(&app);
        let auth = start(&mut browser).await;
        issuer
            .issue(issuer.id_token(&param(&auth, "nonce"), groups))
            .await;

        let reply = callback(&mut browser, &param(&auth, "state")).await;

        if let Some(role) = role {
            assert_eq!(reply.status, StatusCode::FOUND, "{groups:?}");
            assert_eq!(me(&mut browser).await["role"], role, "{groups:?}");
        } else {
            assert_eq!(
                sent_back_to(&reply),
                "/sign-in?error=forbidden",
                "{groups:?}"
            );
            assert_eq!(
                browser.get("/api/me").await.status,
                StatusCode::UNAUTHORIZED
            );
        }
    }
}

#[tokio::test]
async fn a_code_the_issuer_rejects_is_forbidden() {
    let issuer = Issuer::start().await;
    let app = issuer.corral().await;
    let mut browser = browser(&app);
    let auth = start(&mut browser).await;
    issuer
        .answer_token_request(
            ResponseTemplate::new(400).set_body_json(json!({ "error": "invalid_grant" })),
        )
        .await;

    let reply = callback(&mut browser, &param(&auth, "state")).await;

    assert_eq!(sent_back_to(&reply), "/sign-in?error=forbidden");
}

#[tokio::test]
async fn login_still_works_after_the_issuer_rotates_its_signing_key() {
    let mut issuer = Issuer::start().await;
    let app = issuer.corral().await;
    issuer.rotate_key().await;
    let mut browser = browser(&app);
    let auth = start(&mut browser).await;
    issuer
        .issue(issuer.id_token(&param(&auth, "nonce"), Some(&[MEMBERS])))
        .await;

    let reply = callback(&mut browser, &param(&auth, "state")).await;

    assert_eq!(reply.status, StatusCode::FOUND, "{}", reply.text());
    assert_eq!(me(&mut browser).await["handle"], "adam");
}

#[tokio::test]
async fn each_login_takes_the_picture_claim_when_it_is_an_https_url() {
    let mut issuer = Issuer::start().await;
    let app = issuer.corral().await;
    for (claim, picture) in [
        (
            Some("https://idp.example.com/adam.png"),
            json!("https://idp.example.com/adam.png"),
        ),
        (
            Some("https://idp.example.com/new.png"),
            json!("https://idp.example.com/new.png"),
        ),
        (Some("http://idp.example.com/adam.png"), json!(null)),
        (Some("javascript:alert(1)"), json!(null)),
        (Some("/adam.png"), json!(null)),
        (None, json!(null)),
    ] {
        issuer.picture = claim;
        issuer.server.reset().await;
        issuer.serve_discovery().await;
        let mut browser = browser(&app);
        let auth = start(&mut browser).await;
        issuer
            .issue(issuer.id_token(&param(&auth, "nonce"), Some(&[MEMBERS])))
            .await;

        let reply = callback(&mut browser, &param(&auth, "state")).await;

        assert_eq!(
            reply.status,
            StatusCode::FOUND,
            "{claim:?}: {}",
            reply.text()
        );
        assert_eq!(me(&mut browser).await["picture"], picture, "{claim:?}");
    }
}

#[tokio::test]
async fn a_login_returns_to_the_page_it_was_asked_to_when_that_is_on_this_origin() {
    let issuer = Issuer::start().await;
    let app = issuer.corral().await;
    for (next, back_to) in [
        ("%2Fsandboxes%2Fmain%3Ftab%3Dssh", "/sandboxes/main?tab=ssh"),
        ("%2Fadmin%2Fusers", "/admin/users"),
        ("%2F%2Fevil.example%2Fx", "/"),
        ("%2F%5Cevil.example", "/"),
        ("https%3A%2F%2Fevil.example%2F", "/"),
        ("sandboxes", "/"),
        ("", "/"),
    ] {
        issuer.server.reset().await;
        issuer.serve_discovery().await;
        let mut browser = browser(&app);
        let start = browser.get(&format!("/auth/oidc/start?next={next}")).await;
        let auth = Url::parse(start.headers[header::LOCATION].to_str().unwrap()).unwrap();
        issuer
            .issue(issuer.id_token(&param(&auth, "nonce"), Some(&[MEMBERS])))
            .await;

        let done = callback(&mut browser, &param(&auth, "state")).await;

        assert_eq!(done.status, StatusCode::FOUND, "{next}: {}", done.text());
        assert_eq!(done.headers[header::LOCATION], back_to, "{next}");
        assert_eq!(me(&mut browser).await["handle"], "adam", "{next}");
    }
}
