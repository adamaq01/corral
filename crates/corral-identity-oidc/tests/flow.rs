use std::collections::HashMap;

use chrono::{Duration, Utc};
use corral_core::Error;
use corral_core::domain::Principal;
use corral_core::ports::{IdentityProvider, LoginStart};
use corral_identity_oidc::{OidcConfig, OidcProvider};
use openidconnect::core::{
    CoreGenderClaim, CoreJsonWebKeySet, CoreJweContentEncryptionAlgorithm, CoreJwsSigningAlgorithm,
    CoreProviderMetadata, CoreResponseType, CoreRsaPrivateSigningKey, CoreSubjectIdentifierType,
};
use openidconnect::url::{Url, form_urlencoded};
use openidconnect::{
    AdditionalClaims, Audience, AuthUrl, EmptyAdditionalProviderMetadata, EndUserEmail,
    EndUserUsername, IdToken, IdTokenClaims, IssuerUrl, JsonWebKeyId, JsonWebKeySetUrl, Nonce,
    PkceCodeChallenge, PkceCodeVerifier, PrivateSigningKey, ResponseTypes, StandardClaims,
    SubjectIdentifier, TokenUrl,
};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_json::json;
use wiremock::matchers::{basic_auth, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const CLIENT_ID: &str = "corral";
const CLIENT_SECRET: &str = "corral-secret";
const REDIRECT_URL: &str = "https://corral.example/auth/callback";
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
}

impl Issuer {
    async fn start() -> Self {
        let issuer = Self {
            server: MockServer::start().await,
            key: signing_key(FIRST_KEY, "first"),
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

    async fn provider(&self) -> OidcProvider {
        OidcProvider::discover(OidcConfig {
            issuer: self.server.uri(),
            client_id: CLIENT_ID.to_owned(),
            client_secret: SecretString::from(CLIENT_SECRET),
            redirect_url: REDIRECT_URL.to_owned(),
            scopes: OidcConfig::default_scopes(),
        })
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
                .set_email(Some(EndUserEmail::new("adam@example.com".to_owned()))),
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

async fn begin(provider: &OidcProvider) -> (LoginStart, Url) {
    let start = provider.begin().await.unwrap();
    let url = Url::parse(&start.url).unwrap();
    (start, url)
}

#[tokio::test]
async fn login_yields_the_principal_from_the_id_token() {
    let issuer = Issuer::start().await;
    let provider = issuer.provider().await;
    let (start, auth) = begin(&provider).await;
    issuer
        .issue(issuer.id_token(
            &param(&auth, "nonce"),
            Some(&["corral-users", "corral-admins"]),
        ))
        .await;

    let principal = provider
        .complete(start.state, CODE, &param(&auth, "state"))
        .await
        .unwrap();

    assert_eq!(
        principal,
        Principal {
            subject: "sub-123".to_owned(),
            preferred_username: "Adam".to_owned(),
            email: Some("adam@example.com".to_owned()),
            groups: vec!["corral-users".to_owned(), "corral-admins".to_owned()],
        }
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
}

#[tokio::test]
async fn a_different_returned_state_is_forbidden_before_the_code_is_redeemed() {
    let issuer = Issuer::start().await;
    let provider = issuer.provider().await;
    let (start, auth) = begin(&provider).await;
    issuer
        .issue(issuer.id_token(&param(&auth, "nonce"), Some(&["corral-users"])))
        .await;

    let result = provider.complete(start.state, CODE, "forged-state").await;

    assert!(matches!(result, Err(Error::Forbidden)), "{result:?}");
    assert_eq!(issuer.token_requests().await.len(), 0);
}

#[tokio::test]
async fn an_id_token_for_another_nonce_is_forbidden() {
    let issuer = Issuer::start().await;
    let provider = issuer.provider().await;
    let (start, auth) = begin(&provider).await;
    issuer
        .issue(issuer.id_token("another-nonce", Some(&["corral-users"])))
        .await;

    let result = provider
        .complete(start.state, CODE, &param(&auth, "state"))
        .await;

    assert!(matches!(result, Err(Error::Forbidden)), "{result:?}");
}

#[tokio::test]
async fn a_missing_groups_claim_means_no_groups() {
    let issuer = Issuer::start().await;
    let provider = issuer.provider().await;
    let (start, auth) = begin(&provider).await;
    issuer
        .issue(issuer.id_token(&param(&auth, "nonce"), None))
        .await;

    let principal = provider
        .complete(start.state, CODE, &param(&auth, "state"))
        .await
        .unwrap();

    assert_eq!(principal.groups, Vec::<String>::new());
}

#[tokio::test]
async fn a_code_the_issuer_rejects_is_forbidden() {
    let issuer = Issuer::start().await;
    let provider = issuer.provider().await;
    let (start, auth) = begin(&provider).await;
    issuer
        .answer_token_request(
            ResponseTemplate::new(400).set_body_json(json!({ "error": "invalid_grant" })),
        )
        .await;

    let result = provider
        .complete(start.state, CODE, &param(&auth, "state"))
        .await;

    assert!(matches!(result, Err(Error::Forbidden)), "{result:?}");
}

#[tokio::test]
async fn login_still_works_after_the_issuer_rotates_its_signing_key() {
    let mut issuer = Issuer::start().await;
    let provider = issuer.provider().await;
    issuer.rotate_key().await;
    let (start, auth) = begin(&provider).await;
    issuer
        .issue(issuer.id_token(&param(&auth, "nonce"), Some(&["corral-users"])))
        .await;

    let principal = provider
        .complete(start.state, CODE, &param(&auth, "state"))
        .await
        .unwrap();

    assert_eq!(principal.subject, "sub-123");
}
