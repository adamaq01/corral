use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get};
use chrono::{DateTime, Utc};
use cookie::SameSite;
use corral_api::testing::{Browser, Reply, api, trusting_provider};
use corral_api::{
    ApiError, ClientAddress, CurrentAdmin, Identities, LoginKind, PathParam, ProviderCookies,
    ProviderDescription, ProviderRoutes, router,
};
use corral_core::app::Setup;
use corral_core::domain::{Handle, Role, SecretId, UserId};
use corral_core::fakes::{ADAM_KEY, EngineCall, MAX_SANDBOXES, PROVIDER, World, exposed, links};
use corral_core::ports::{EventKind, Identity, SecretStore, UserRepo, VariableMap};
use corral_core::{Error, Result};
use secrecy::SecretSlice;
use serde_json::{Value, json};
use tower::ServiceExt;

const STEP: &str = "step";
const STEP_SECRET: &str = "the-private-step-secret";

/// The trusting provider as the world's provider, with a two-step login and the toolkit's other
/// extractors on show.
fn stub() -> ProviderRoutes {
    let trusting = trusting_provider(PROVIDER);
    ProviderRoutes {
        description: ProviderDescription {
            label: "Stub".to_owned(),
            kind: LoginKind::Redirect,
            login: "/auth/test/start".to_owned(),
            accounts: None,
            opkssh: None,
        },
        routes: trusting
            .routes
            .route("/start", get(stub_start))
            .route("/finish", get(stub_finish))
            .route("/address", get(stub_address))
            .route("/identities/{subject}", delete(stub_delete)),
        ..trusting
    }
}

async fn stub_start(cookies: ProviderCookies) -> impl IntoResponse {
    let cookies = cookies.set(STEP, &STEP_SECRET, Duration::from_secs(600));
    (cookies, StatusCode::NO_CONTENT)
}

async fn stub_finish(cookies: ProviderCookies, identities: Identities) -> Response {
    let Some(secret) = cookies.get::<String>(STEP) else {
        return ApiError::new(StatusCode::BAD_REQUEST, "no login in progress").into_response();
    };
    assert_eq!(secret, STEP_SECRET);
    let identity = Identity {
        subject: "sub-1".to_owned(),
        username: "Adam".to_owned(),
        email: Some("adam@example.com".to_owned()),
        picture: None,
        role: Some(Role::Member),
    };
    let cookies = cookies.remove(STEP);
    match identities.sign_in(identity).await {
        Ok(signed_in) => (
            StatusCode::FOUND,
            cookies,
            signed_in,
            [(header::LOCATION, "/")],
        )
            .into_response(),
        Err(error) => (cookies, error).into_response(),
    }
}

async fn stub_address(ClientAddress(address): ClientAddress) -> String {
    address.to_string()
}

async fn stub_delete(
    CurrentAdmin(admin): CurrentAdmin,
    identities: Identities,
    PathParam(subject): PathParam<String>,
) -> Result<axum::Json<Value>, ApiError> {
    let had_user = identities.delete(&admin, &subject).await?;
    Ok(axum::Json(json!({ "hadUser": had_user })))
}

struct BrokenVault;

#[async_trait]
impl SecretStore for BrokenVault {
    async fn put(&self, _: UserId, _: SecretId, _: &SecretSlice<u8>) -> Result<()> {
        Err(Error::internal(
            "vault",
            "vault at 10.9.8.7 refused token s.internal",
        ))
    }

    async fn get(&self, _: UserId, _: &[SecretId]) -> Result<BTreeMap<SecretId, SecretSlice<u8>>> {
        Ok(BTreeMap::new())
    }

    async fn remove(&self, _: UserId, _: SecretId) -> Result<bool> {
        Ok(false)
    }

    async fn remove_all(&self, _: UserId) -> Result<()> {
        Ok(())
    }

    async fn legacy_credentials(&self, _: UserId) -> Result<VariableMap> {
        Ok(VariableMap::new())
    }

    async fn forget_legacy_credentials(&self, _: UserId) -> Result<()> {
        Ok(())
    }
}

#[derive(Default)]
struct Options {
    secure: bool,
    trust_forwarded_for: bool,
    secrets: Option<Arc<dyn SecretStore>>,
}

struct Corral {
    app: Router,
    world: World,
}

impl Options {
    fn build(self) -> Corral {
        let world = World::new();
        let mut api = api(&world);
        api.secure_cookies = self.secure;
        api.trust_forwarded_for = self.trust_forwarded_for;
        if let Some(secrets) = self.secrets {
            api.setup = Setup::new(
                world.setup_repo.clone(),
                secrets,
                world.sandbox_repo.clone(),
                world.users.clone(),
                links(),
            );
        }
        Corral {
            app: router(api, vec![stub()]),
            world,
        }
    }
}

impl Corral {
    fn new() -> Self {
        Options::default().build()
    }

    fn browser(&self) -> Browser {
        self.browser_at("192.0.2.10:50000")
    }

    fn browser_at(&self, peer: &str) -> Browser {
        Browser::new(&self.app, peer)
    }

    async fn signed_in(&self, handle: &str, role: Role) -> Browser {
        let mut browser = self.browser();
        let reply = sign_in(&mut browser, handle, Some(role)).await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
        browser
    }
}

async fn sign_in(browser: &mut Browser, handle: &str, role: Option<Role>) -> Reply {
    let login = json!({ "subject": handle, "username": handle, "role": role });
    browser.post("/auth/test/login", login).await
}

#[tokio::test]
async fn a_provider_sign_in_starts_a_session_and_me_returns_the_handle() {
    let corral = Corral::new();
    let mut browser = corral.browser();
    assert_eq!(
        browser.get("/api/me").await.status,
        StatusCode::UNAUTHORIZED
    );

    let reply = sign_in(&mut browser, "adam", Some(Role::Member)).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.json(),
        json!({
            "handle": "adam",
            "email": null,
            "picture": null,
            "role": "member",
            "provider": "test",
        })
    );
    let me = browser.get("/api/me").await;
    assert_eq!(me.status, StatusCode::OK);
    assert_eq!(me.json()["handle"], "adam");

    let logout = browser.post("/auth/logout", json!({})).await;
    assert_eq!(logout.status, StatusCode::NO_CONTENT);
    assert_eq!(
        browser.get("/api/me").await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn the_providers_picture_is_shown_for_the_user_as_of_their_last_login() {
    let corral = Corral::new();
    let mut root = corral.signed_in("root", Role::Admin).await;
    let mut adam = corral.browser();
    let picture = "https://idp.example.com/adam.png";
    for (sent, shown) in [(json!(picture), json!(picture)), (json!(null), json!(null))] {
        let login = json!({
            "subject": "adam", "username": "adam", "role": "member", "picture": sent,
        });
        let reply = adam.post("/auth/test/login", login).await;
        assert_eq!(reply.json()["picture"], shown);
        assert_eq!(adam.get("/api/me").await.json()["picture"], shown);
        let users = root.get("/api/admin/users").await.json();
        let pictures: Vec<_> = users
            .as_array()
            .unwrap()
            .iter()
            .map(|user| (user["handle"].clone(), user["picture"].clone()))
            .collect();
        assert_eq!(
            pictures,
            [(json!("root"), json!(null)), (json!("adam"), shown.clone())]
        );
    }
}

#[tokio::test]
async fn a_person_without_a_role_gets_no_session() {
    let corral = Corral::new();
    let mut browser = corral.browser();
    let refused = sign_in(&mut browser, "adam", None).await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN);
    assert_eq!(refused.json(), json!({ "error": "forbidden" }));
    assert!(!refused.headers.contains_key(header::SET_COOKIE));
    assert_eq!(
        browser.get("/api/me").await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn providers_are_listed_as_they_describe_themselves() {
    let world = World::new();
    let other = ProviderRoutes {
        id: "other".to_owned(),
        description: ProviderDescription {
            label: "Other".to_owned(),
            kind: LoginKind::Password,
            login: "/auth/other/login".to_owned(),
            accounts: Some("/auth/other/accounts".to_owned()),
            opkssh: None,
        },
        routes: Router::new(),
    };
    let app = router(api(&world), vec![stub(), other]);
    let mut browser = Browser::new(&app, "192.0.2.10:50000");
    assert_eq!(
        browser.get("/auth/providers").await.json(),
        json!([
            {
                "id": "test",
                "label": "Stub",
                "kind": "redirect",
                "login": "/auth/test/start",
                "accounts": null,
                "opkssh": null,
            },
            {
                "id": "other",
                "label": "Other",
                "kind": "password",
                "login": "/auth/other/login",
                "accounts": "/auth/other/accounts",
                "opkssh": null,
            },
        ])
    );
    for uri in ["/auth/other/login", "/auth/nope/start", "/auth/test/nope"] {
        assert_eq!(
            browser.get(uri).await.status,
            StatusCode::NOT_FOUND,
            "{uri}"
        );
    }

    let none = router(api(&world), Vec::new());
    let mut browser = Browser::new(&none, "192.0.2.10:50000");
    assert_eq!(browser.get("/auth/providers").await.json(), json!([]));
    assert_eq!(
        browser.get("/auth/test/start").await.status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn unknown_api_and_auth_paths_are_json_404s_whatever_serves_the_rest() {
    let world = World::new();
    let app = router(api(&world), vec![stub()]).fallback(|| async { "the rest" });
    let mut browser = Browser::new(&app, "192.0.2.10:50000");
    for uri in [
        "/api",
        "/api/",
        "/api/nope",
        "/api/sandboxes/main/nope",
        "/api/admin",
        "/auth",
        "/auth/",
        "/auth/nope",
        "/auth/nope/start",
        "/auth/test",
        "/auth/test/",
        "/auth/test/nope",
    ] {
        for method in ["GET", "POST", "DELETE"] {
            let reply = browser.call(method, uri, None).await;
            assert_eq!(reply.status, StatusCode::NOT_FOUND, "{method} {uri}");
            assert_eq!(
                reply.json(),
                json!({ "error": "not found" }),
                "{method} {uri}"
            );
        }
    }
    assert_eq!(
        browser.get("/api/me").await.status,
        StatusCode::UNAUTHORIZED
    );
    for uri in ["/", "/apis", "/authors", "/sandboxes/main"] {
        assert_eq!(browser.get(uri).await.text(), "the rest", "{uri}");
    }
}

#[tokio::test]
async fn provider_cookies_carry_a_login_between_steps_unreadable() {
    let corral = Corral::new();
    let mut browser = corral.browser();
    let start = browser.get("/auth/test/start").await;
    assert_eq!(start.status, StatusCode::NO_CONTENT);
    let step = start.set_cookie("corral_test_step");
    assert!(!step.value().contains("private"), "{step}");
    assert_eq!(step.max_age(), Some(cookie::time::Duration::minutes(10)));
    assert_eq!(step.path(), Some("/auth/test"));

    let done = browser.get("/auth/test/finish").await;
    assert_eq!(done.status, StatusCode::FOUND, "{}", done.text());
    assert_eq!(done.headers[header::LOCATION], "/");
    assert!(!browser.cookies.contains_key("corral_test_step"));
    assert_eq!(
        browser.get("/api/me").await.json(),
        json!({
            "handle": "adam",
            "email": "adam@example.com",
            "picture": null,
            "role": "member",
            "provider": "test",
        })
    );

    let replay = browser.get("/auth/test/finish").await;
    assert_eq!(replay.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_provider_cookie_and_a_session_cannot_stand_in_for_each_other() {
    let corral = Corral::new();
    let mut browser = corral.browser();
    browser.get("/auth/test/start").await;
    let step = browser.cookies["corral_test_step"].clone();
    let mut forger = corral.browser();
    forger.cookies.insert("corral_session".to_owned(), step);
    assert_eq!(forger.get("/api/me").await.status, StatusCode::UNAUTHORIZED);

    let mut adam = corral.signed_in("adam", Role::Member).await;
    let session = adam.cookies["corral_session"].clone();
    adam.cookies.insert("corral_test_step".to_owned(), session);
    assert_eq!(
        adam.get("/auth/test/finish").await.status,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn cookies_lack_secure_when_base_url_is_http() {
    for secure in [false, true] {
        let corral = Options {
            secure,
            ..Options::default()
        }
        .build();
        let mut browser = corral.browser();
        let session = sign_in(&mut browser, "adam", Some(Role::Member))
            .await
            .set_cookie("corral_session");
        let step = browser
            .get("/auth/test/start")
            .await
            .set_cookie("corral_test_step");
        assert_eq!(session.path(), Some("/"));
        for cookie in [session, step] {
            assert_eq!(cookie.secure(), secure.then_some(true), "{cookie}");
            assert_eq!(cookie.http_only(), Some(true), "{cookie}");
            assert_eq!(cookie.same_site(), Some(SameSite::Lax), "{cookie}");
        }
    }
}

#[tokio::test]
async fn the_client_address_is_the_peer_unless_a_trusted_proxy_says_otherwise() {
    async fn address(corral: &Corral, forwarded_for: Option<&str>) -> String {
        let mut request = Request::get("/auth/test/address");
        if let Some(forwarded_for) = forwarded_for {
            request = request.header("x-forwarded-for", forwarded_for);
        }
        let mut browser = corral.browser_at("10.0.0.2:40000");
        browser
            .send(request.body(Body::empty()).unwrap())
            .await
            .text()
    }

    let untrusting = Corral::new();
    for forwarded_for in [None, Some("203.0.113.7"), Some("198.51.100.9, 203.0.113.7")] {
        assert_eq!(address(&untrusting, forwarded_for).await, "10.0.0.2");
    }
    let trusting = Options {
        trust_forwarded_for: true,
        ..Options::default()
    }
    .build();
    for (forwarded_for, client) in [
        (None, "10.0.0.2"),
        (Some("203.0.113.7"), "203.0.113.7"),
        (Some("198.51.100.9, 203.0.113.7"), "203.0.113.7"),
        (Some("not an address"), "10.0.0.2"),
    ] {
        assert_eq!(
            address(&trusting, forwarded_for).await,
            client,
            "{forwarded_for:?}"
        );
    }
}

#[tokio::test]
async fn a_missing_peer_address_fails_closed() {
    let corral = Corral::new();
    let request = Request::get("/auth/test/address")
        .body(Body::empty())
        .unwrap();
    let response = corral.app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn admins_delete_identities_through_their_provider() {
    let corral = Corral::new();
    let mut root = corral.signed_in("root", Role::Admin).await;
    let mut adam = owning_everything(&corral, "adam").await;
    let deleted = root
        .call("DELETE", "/auth/test/identities/adam", None)
        .await;
    assert_eq!(deleted.json(), json!({ "hadUser": true }));
    assert_eq!(adam.get("/api/me").await.status, StatusCode::FORBIDDEN);
    assert_eq!(root.get("/api/admin/sandboxes").await.json(), json!([]));
    let never = root
        .call("DELETE", "/auth/test/identities/never", None)
        .await;
    assert_eq!(never.json(), json!({ "hadUser": false }));
    assert_eq!(corral.world.provider.forgotten(), ["adam", "never"]);

    let mut eve = corral.signed_in("eve", Role::Member).await;
    let refused = eve.call("DELETE", "/auth/test/identities/root", None).await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN);
    assert_eq!(root.get("/api/me").await.status, StatusCode::OK);
    owns_nothing_after_signing_up_again(&corral, "adam").await;
}

const API_ROUTES: &[(&str, &str)] = &[
    ("GET", "/api/me"),
    ("DELETE", "/api/me"),
    ("PUT", "/api/me/sandbox-defaults"),
    ("GET", "/api/me/delete-from-inside"),
    ("PUT", "/api/me/delete-from-inside"),
    ("GET", "/api/catalog"),
    ("GET", "/api/sandboxes"),
    ("POST", "/api/sandboxes"),
    ("GET", "/api/sandboxes/main"),
    ("DELETE", "/api/sandboxes/main"),
    ("POST", "/api/sandboxes/main/wake"),
    ("POST", "/api/sandboxes/main/stop"),
    ("POST", "/api/sandboxes/main/restart"),
    ("POST", "/api/sandboxes/main/default"),
    ("DELETE", "/api/sandboxes/main/deletion"),
    ("GET", "/api/keys"),
    ("POST", "/api/keys"),
    ("DELETE", "/api/keys/00000000-0000-0000-0000-000000000000"),
    ("GET", "/api/setup"),
    ("POST", "/api/setup"),
    ("GET", "/api/setup/00000000-0000-0000-0000-000000000000"),
    ("PUT", "/api/setup/00000000-0000-0000-0000-000000000000"),
    ("DELETE", "/api/setup/00000000-0000-0000-0000-000000000000"),
    ("GET", "/api/sandboxes/main/setup"),
    (
        "PUT",
        "/api/sandboxes/main/setup/00000000-0000-0000-0000-000000000000",
    ),
    (
        "DELETE",
        "/api/sandboxes/main/setup/00000000-0000-0000-0000-000000000000",
    ),
    (
        "PUT",
        "/api/sandboxes/main/setup/00000000-0000-0000-0000-000000000000/override",
    ),
    (
        "DELETE",
        "/api/sandboxes/main/setup/00000000-0000-0000-0000-000000000000/override",
    ),
    (
        "GET",
        "/api/sandboxes/main/setup/00000000-0000-0000-0000-000000000000/preview",
    ),
];

const ADMIN_ROUTES: &[(&str, &str, Option<&str>)] = &[
    ("GET", "/api/admin/users", None),
    (
        "DELETE",
        "/api/admin/users/00000000-0000-0000-0000-000000000000",
        None,
    ),
    (
        "POST",
        "/api/admin/users/00000000-0000-0000-0000-000000000000/enable",
        None,
    ),
    (
        "POST",
        "/api/admin/users/00000000-0000-0000-0000-000000000000/disable",
        None,
    ),
    (
        "PUT",
        "/api/admin/users/00000000-0000-0000-0000-000000000000/allowance",
        Some(r#"{"maxSandboxes": 9}"#),
    ),
    ("GET", "/api/admin/sandboxes", None),
    ("GET", "/api/admin/sandboxes/adam/main", None),
    ("DELETE", "/api/admin/sandboxes/adam/main", None),
    ("POST", "/api/admin/sandboxes/adam/main/stop", None),
    ("POST", "/api/admin/sandboxes/adam/main/restart", None),
    ("POST", "/api/admin/sandboxes/adam/main/wake", None),
    ("GET", "/api/admin/settings/catalog", None),
    (
        "PUT",
        "/api/admin/settings/catalog",
        Some(r#"{"images": [], "sizes": []}"#),
    ),
    ("GET", "/api/admin/settings/limits", None),
    (
        "PUT",
        "/api/admin/settings/limits",
        Some(r#"{"maxRunning": 9}"#),
    ),
    ("GET", "/api/admin/settings/registries", None),
    ("PUT", "/api/admin/settings/registries", Some("[]")),
    ("GET", "/api/admin/settings/shared-setup", None),
    ("POST", "/api/admin/settings/shared-setup", Some("{}")),
    (
        "GET",
        "/api/admin/settings/shared-setup/00000000-0000-0000-0000-000000000000",
        None,
    ),
    (
        "PUT",
        "/api/admin/settings/shared-setup/00000000-0000-0000-0000-000000000000",
        Some("{}"),
    ),
    (
        "DELETE",
        "/api/admin/settings/shared-setup/00000000-0000-0000-0000-000000000000",
        None,
    ),
];

#[tokio::test]
async fn unauthenticated_api_calls_are_401() {
    let corral = Corral::new();
    let routes = API_ROUTES
        .iter()
        .copied()
        .chain(ADMIN_ROUTES.iter().map(|&(method, uri, _)| (method, uri)))
        .chain([("DELETE", "/auth/test/identities/adam")]);
    let mut anonymous = corral.browser();
    let mut forger = corral.browser();
    forger
        .cookies
        .insert("corral_session".to_owned(), "forged".to_owned());
    for (method, uri) in routes {
        for browser in [&mut anonymous, &mut forger] {
            let reply = browser.call(method, uri, Some(json!({}))).await;
            assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{method} {uri}");
            assert_eq!(reply.json(), json!({ "error": "not signed in" }));
        }
    }
}

#[tokio::test]
async fn a_disabled_users_session_is_403() {
    let corral = Corral::new();
    let mut root = corral.signed_in("root", Role::Admin).await;
    let mut adam = corral.signed_in("adam", Role::Member).await;
    let users = root.get("/api/admin/users").await.json();
    let adam_id = users
        .as_array()
        .unwrap()
        .iter()
        .find(|user| user["handle"] == "adam")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let disabled = root
        .post(&format!("/api/admin/users/{adam_id}/disable"), json!({}))
        .await;
    assert_eq!(disabled.json()["status"], "disabled");
    for (method, uri) in API_ROUTES {
        let reply = adam.call(method, uri, Some(json!({}))).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "{method} {uri}");
    }

    root.post(&format!("/api/admin/users/{adam_id}/enable"), json!({}))
        .await;
    assert_eq!(adam.get("/api/me").await.status, StatusCode::OK);
}

#[tokio::test]
async fn create_then_list_shows_the_ssh_username() {
    let corral = Corral::new();
    let mut adam = corral.signed_in("adam", Role::Member).await;
    let before = Utc::now();
    let created = adam.post("/api/sandboxes", json!({ "name": "main" })).await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.text());
    let created = created.json();
    let created_at: DateTime<Utc> = created["createdAt"].as_str().unwrap().parse().unwrap();
    assert!((before..=Utc::now()).contains(&created_at), "{created_at}");
    let expected = json!({
        "name": "main",
        "image": "base",
        "size": "small",
        "isDefault": true,
        "state": "running",
        "ssh": { "username": "main.adam", "host": "corral.example.com", "port": 48291 },
        "createdAt": created["createdAt"],
        "staleSetup": false,
        "deleteAt": null,
        "idle": {
            "timeoutSecs": 1800,
            "chosen": false,
            "defaultTimeoutSecs": 1800,
            "minTimeoutSecs": 60,
            "maxTimeoutSecs": 4 * 3600,
            "keepAwakeUntil": null,
            "maxKeepAwakeSecs": 24 * 3600,
        },
        "repository": null,
    });
    assert_eq!(created, expected);
    assert_eq!(adam.get("/api/sandboxes").await.json(), json!([expected]));
    assert_eq!(adam.get("/api/sandboxes/main").await.json(), expected);

    assert_eq!(
        adam.get("/api/catalog").await.json(),
        json!({
            "images": [{ "key": "base", "label": "BASE" }],
            "sizes": [{ "key": "small", "label": "SMALL" }, { "key": "large", "label": "LARGE" }],
            "defaultImage": "base",
            "defaultSize": "small",
            "defaultsWithdrawn": false,
            "maxSandboxes": MAX_SANDBOXES,
            "ssh": { "host": "corral.example.com", "port": 48291 },
        })
    );
}

#[tokio::test]
async fn an_image_reference_is_refused_where_users_run_only_the_catalogs_images() {
    let corral = Corral::new();
    let mut adam = corral.signed_in("adam", Role::Member).await;
    let refused = adam
        .post(
            "/api/sandboxes",
            json!({ "name": "main", "imageReference": "debian:13" }),
        )
        .await;
    assert_eq!(
        refused.status,
        StatusCode::BAD_REQUEST,
        "{}",
        refused.text()
    );
    assert!(
        refused.text().contains("only the catalog's images"),
        "{}",
        refused.text()
    );
    assert_eq!(adam.get("/api/sandboxes").await.json(), json!([]));
}

#[tokio::test]
async fn a_sandbox_is_made_with_chosen_entries_and_a_repository() {
    let corral = Corral::new();
    let mut adam = corral.signed_in("adam", Role::Member).await;
    adam.post("/api/setup", variable("GH_TOKEN", "ghp_1", false))
        .await;
    let mut extra = variable("EXTRA", "x", false);
    extra["applyToNew"] = json!(false);
    let extra = adam.post("/api/setup", extra).await.json();
    let repository = json!({ "url": "https://github.com/adamaq01/corral.git", "branch": "main" });

    let created = adam
        .post(
            "/api/sandboxes",
            json!({ "name": "main", "setup": [extra["id"]], "repository": repository }),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.text());
    assert_eq!(created.json()["repository"], repository);
    assert_eq!(
        adam.get("/api/sandboxes/main").await.json()["repository"],
        repository
    );
    let attached = |listed: Value| -> Vec<Value> {
        listed
            .as_array()
            .unwrap()
            .iter()
            .map(|attached| attached["entry"]["target"].clone())
            .collect()
    };
    assert_eq!(
        attached(adam.get("/api/sandboxes/main/setup").await.json()),
        [json!("EXTRA")]
    );

    let plain = adam
        .post("/api/sandboxes", json!({ "name": "plain" }))
        .await;
    assert_eq!(plain.json()["repository"], Value::Null);
    assert_eq!(
        attached(adam.get("/api/sandboxes/plain/setup").await.json()),
        [json!("GH_TOKEN")]
    );
}

#[tokio::test]
async fn a_repository_url_with_a_password_is_refused() {
    let corral = Corral::new();
    let mut adam = corral.signed_in("adam", Role::Member).await;
    let refused = adam
        .post(
            "/api/sandboxes",
            json!({
                "name": "main",
                "repository": { "url": "https://me:tok3n@github.com/me/x.git" },
            }),
        )
        .await;
    assert_eq!(
        refused.status,
        StatusCode::BAD_REQUEST,
        "{}",
        refused.text()
    );
    let said = refused.json()["error"].as_str().unwrap().to_owned();
    assert!(
        said.contains(
            "the repository URL carries a login; put the token in Setup, as a git token or \
             SSH key, and leave it out of the URL"
        ),
        "{said}"
    );
    assert!(!said.contains("tok3n"), "{said}");
    assert_eq!(adam.get("/api/sandboxes").await.json(), json!([]));
    assert!(corral.world.engine.calls().is_empty());
}

#[tokio::test]
async fn an_unknown_setup_id_is_refused() {
    let corral = Corral::new();
    let mut adam = corral.signed_in("adam", Role::Member).await;
    let mut eve = corral.signed_in("eve", Role::Member).await;
    let eves = eve
        .post("/api/setup", variable("GH_TOKEN", "ghp_eve", true))
        .await
        .json();
    let eves = eves["id"].as_str().unwrap();
    for id in ["00000000-0000-0000-0000-000000000000", eves] {
        let refused = adam
            .post("/api/sandboxes", json!({ "name": "main", "setup": [id] }))
            .await;
        assert_eq!(
            refused.status,
            StatusCode::BAD_REQUEST,
            "{}",
            refused.text()
        );
        assert!(
            refused
                .text()
                .contains(&format!("there is no setup entry {id}")),
            "{}",
            refused.text()
        );
    }
    assert_eq!(adam.get("/api/sandboxes").await.json(), json!([]));
}

#[tokio::test]
async fn a_users_sandbox_defaults_preselect_and_fill_in_new_sandboxes() {
    let corral = Corral::new();
    corral.world.offer_tools().await;
    let mut adam = corral.signed_in("adam", Role::Member).await;
    let mut eve = corral.signed_in("eve", Role::Member).await;
    let defaults_of = async |browser: &mut Browser| {
        let catalog = browser.get("/api/catalog").await.json();
        assert_eq!(catalog["defaultsWithdrawn"], json!(false));
        json!([catalog["defaultImage"], catalog["defaultSize"]])
    };
    assert_eq!(defaults_of(&mut adam).await, json!(["base", "small"]));

    let set = adam
        .call(
            "PUT",
            "/api/me/sandbox-defaults",
            Some(json!({ "image": "tools", "size": "large" })),
        )
        .await;
    assert_eq!(set.status, StatusCode::NO_CONTENT, "{}", set.text());
    assert_eq!(defaults_of(&mut adam).await, json!(["tools", "large"]));
    assert_eq!(defaults_of(&mut eve).await, json!(["base", "small"]));
    let created = adam.post("/api/sandboxes", json!({ "name": "main" })).await;
    let created = created.json();
    assert_eq!(
        json!([created["image"], created["size"]]),
        json!(["tools", "large"])
    );

    for refused in [
        json!({ "image": "nope", "size": "large" }),
        json!({ "image": "tools", "size": "nope" }),
        json!({ "image": "tools" }),
    ] {
        let reply = adam
            .call("PUT", "/api/me/sandbox-defaults", Some(refused.clone()))
            .await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{refused}");
    }
    assert_eq!(defaults_of(&mut adam).await, json!(["tools", "large"]));
}

#[tokio::test]
async fn sandbox_actions_reach_the_engine() {
    let corral = Corral::new();
    let mut adam = corral.signed_in("adam", Role::Member).await;
    for name in ["main", "spare"] {
        adam.post("/api/sandboxes", json!({ "name": name })).await;
    }
    let stop = adam.call("POST", "/api/sandboxes/main/stop", None).await;
    assert_eq!(stop.status, StatusCode::NO_CONTENT);
    assert_eq!(
        adam.get("/api/sandboxes/main").await.json()["state"],
        "sleeping"
    );

    corral.world.engine.set_waking(true);
    let waking = adam.call("POST", "/api/sandboxes/main/wake", None).await;
    assert_eq!(waking.json(), json!({ "ready": false }));
    corral.world.engine.set_waking(false);
    let ready = adam.call("POST", "/api/sandboxes/main/wake", None).await;
    assert_eq!(ready.json(), json!({ "ready": true }));
    let restart = adam.call("POST", "/api/sandboxes/main/restart", None).await;
    assert_eq!(restart.status, StatusCode::NO_CONTENT);

    let default = adam
        .call("POST", "/api/sandboxes/spare/default", None)
        .await;
    assert_eq!(default.status, StatusCode::NO_CONTENT);
    let defaults: Vec<Value> = adam
        .get("/api/sandboxes")
        .await
        .json()
        .as_array()
        .unwrap()
        .iter()
        .map(|sandbox| json!([sandbox["name"], sandbox["isDefault"]]))
        .collect();
    assert_eq!(defaults, [json!(["main", false]), json!(["spare", true])]);

    let deleted = adam.call("DELETE", "/api/sandboxes/spare", None).await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    assert_eq!(
        adam.get("/api/sandboxes/spare").await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        adam.get("/api/sandboxes/main").await.json()["isDefault"],
        true
    );
}

fn variable(name: &str, value: &str, secret: bool) -> Value {
    json!({
        "kind": "variable",
        "target": name,
        "secret": secret,
        "applyToNew": true,
        "value": { "text": value },
    })
}

#[tokio::test]
async fn a_running_sandbox_is_stale_from_a_setup_change_until_it_restarts() {
    let corral = Corral::new();
    let mut adam = corral.signed_in("adam", Role::Member).await;
    let created = adam
        .post("/api/setup", variable("GH_TOKEN", "ghp_1", false))
        .await
        .json();
    adam.post("/api/sandboxes", json!({ "name": "main" })).await;
    let main = "/api/sandboxes/main";
    assert_eq!(adam.get(main).await.json()["staleSetup"], false);

    let changed = adam
        .call(
            "PUT",
            &format!("/api/setup/{}", created["id"].as_str().unwrap()),
            Some(json!({
                "kind": "variable",
                "target": "GH_TOKEN",
                "secret": false,
                "template": false,
                "executable": false,
                "applyToNew": true,
                "value": { "text": "ghp_2" },
            })),
        )
        .await;
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.text());
    assert_eq!(changed.json()["value"]["text"], "ghp_2");
    assert_eq!(adam.get(main).await.json()["staleSetup"], true);
    assert_eq!(
        adam.get("/api/sandboxes").await.json()[0]["staleSetup"],
        true
    );
    adam.call("POST", "/api/sandboxes/main/restart", None).await;
    assert_eq!(adam.get(main).await.json()["staleSetup"], false);
}

fn file(target: &str, secret: bool, value: &Value) -> Value {
    json!({
        "kind": "file",
        "target": target,
        "secret": secret,
        "applyToNew": false,
        "value": value,
    })
}

fn shown(entry: &Value, fields: &[&str]) -> Vec<Value> {
    fields
        .iter()
        .map(|field| entry["value"][*field].clone())
        .collect()
}

#[tokio::test]
async fn secret_values_never_come_back_and_show_only_a_glimpse() {
    let corral = Corral::new();
    let mut adam = corral.signed_in("adam", Role::Member).await;
    let created = adam
        .post(
            "/api/setup",
            variable("GH_TOKEN", "ghp_s3cr3t-value-0123456789Xz", true),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.text());
    let entry = created.json();
    assert_eq!(
        [
            &entry["kind"],
            &entry["target"],
            &entry["secret"],
            &entry["applyToNew"]
        ],
        [
            &json!("variable"),
            &json!("GH_TOKEN"),
            &json!(true),
            &json!(true)
        ]
    );
    assert_eq!(
        shown(&entry, &["shown", "length", "head", "tail", "size"]),
        [
            json!("masked"),
            json!(29),
            json!("ghp_"),
            json!("Xz"),
            json!(29)
        ]
    );
    assert!(entry["value"]["setAt"].is_string());
    let short = adam
        .post("/api/setup", variable("PIN", "s3cr3t", true))
        .await
        .json();
    assert_eq!(
        shown(&short, &["shown", "length", "head"]),
        [json!("masked"), json!(6), Value::Null]
    );
    let key = adam
        .post(
            "/api/setup",
            file(".ssh/id", true, &json!({ "text": "s3cr3t key" })),
        )
        .await
        .json();
    assert_eq!(
        shown(&key, &["shown", "size"]),
        [json!("hidden"), json!(10)]
    );

    let mut replies = vec![adam.get("/api/setup").await];
    replies.push(
        adam.get(&format!("/api/setup/{}", entry["id"].as_str().unwrap()))
            .await,
    );
    replies.push(adam.post("/api/sandboxes", json!({ "name": "main" })).await);
    replies.push(adam.get("/api/sandboxes/main/setup").await);
    replies.push(
        adam.post("/api/setup", variable("OTHER", "s3cr3t\nvalue", true))
            .await,
    );
    for reply in &replies {
        assert!(!reply.text().contains("s3cr3t"), "{}", reply.text());
    }
    assert_eq!(replies[4].status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn plain_values_show_in_full_and_entries_list_variables_first() {
    let corral = Corral::new();
    let mut adam = corral.signed_in("adam", Role::Member).await;
    let binary = adam
        .post(
            "/api/setup",
            file("blob", false, &json!({ "base64": "AP8=" })),
        )
        .await
        .json();
    assert_eq!(
        shown(&binary, &["shown", "base64", "size"]),
        [json!("binary"), json!("AP8="), json!(2)]
    );
    let text = adam
        .post(
            "/api/setup",
            file("notes", false, &json!({ "base64": "aGk=" })),
        )
        .await
        .json();
    assert_eq!(
        shown(&text, &["shown", "text"]),
        [json!("text"), json!("hi")]
    );
    let plain = adam
        .post("/api/setup", variable("EDITOR", "vim", false))
        .await
        .json();
    assert_eq!(
        shown(&plain, &["shown", "text"]),
        [json!("text"), json!("vim")]
    );
    let listed: Vec<Value> = adam
        .get("/api/setup")
        .await
        .json()
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["target"].clone())
        .collect();
    assert_eq!(listed, ["EDITOR", "blob", "notes"]);

    let id = binary["id"].as_str().unwrap();
    let removed = adam.call("DELETE", &format!("/api/setup/{id}"), None).await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT);
    let again = adam.call("DELETE", &format!("/api/setup/{id}"), None).await;
    assert_eq!(again.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn entries_their_target_cannot_take_are_refused_with_the_reason() {
    let corral = Corral::new();
    let mut adam = corral.signed_in("adam", Role::Member).await;
    adam.post("/api/setup", variable("GH_TOKEN", "x", false))
        .await;
    for (body, status, reason) in [
        (
            variable("gh-token", "x", false),
            StatusCode::BAD_REQUEST,
            "variable name",
        ),
        (
            variable("PATH", "x", false),
            StatusCode::BAD_REQUEST,
            "reserved",
        ),
        (
            file("../outside", false, &json!({ "text": "x" })),
            StatusCode::BAD_REQUEST,
            "inside the home",
        ),
        (
            file("bad", false, &json!({ "base64": "not base64!" })),
            StatusCode::BAD_REQUEST,
            "base64",
        ),
        (
            file("big", false, &json!({ "text": "x".repeat(256 * 1024 + 1) })),
            StatusCode::UNPROCESSABLE_ENTITY,
            "256 KiB",
        ),
        (
            variable("GH_TOKEN", "again", false),
            StatusCode::CONFLICT,
            "you already have variable GH_TOKEN",
        ),
        (
            variable("BIG", &"x".repeat(128 * 1024), false),
            StatusCode::UNPROCESSABLE_ENTITY,
            "128 KiB",
        ),
    ] {
        let reply = adam.post("/api/setup", body).await;
        assert_eq!(reply.status, status, "{}", reply.text());
        assert!(reply.text().contains(reason), "{}", reply.text());
    }
}

/// Adam's main sandbox with a secret `GH_TOKEN`, a file `~/tool` and a template at
/// `~/tool/config.toml` naming the variable, none attached, and each entry's path under main.
async fn three_entries(corral: &Corral) -> (Browser, [String; 3]) {
    let mut adam = corral.signed_in("adam", Role::Member).await;
    adam.post("/api/sandboxes", json!({ "name": "main" })).await;
    let mut paths = Vec::new();
    for mut body in [
        variable("GH_TOKEN", "ghp_everyone", true),
        file("~/tool", false, &json!({ "text": "x" })),
        json!({
            "kind": "file",
            "target": "tool/config.toml",
            "secret": false,
            "template": true,
            "value": { "text": "token = \"{{ GH_TOKEN }}\"" },
        }),
    ] {
        body["applyToNew"] = json!(false);
        let created = adam.post("/api/setup", body).await;
        assert_eq!(created.status, StatusCode::CREATED, "{}", created.text());
        paths.push(format!(
            "/api/sandboxes/main/setup/{}",
            created.json()["id"].as_str().unwrap()
        ));
    }
    (adam, paths.try_into().unwrap())
}

#[tokio::test]
async fn a_sandbox_attaches_entries_that_fit_it() {
    let corral = Corral::new();
    let (mut adam, [token, other, config]) = three_entries(&corral).await;
    let mut eve = corral.signed_in("eve", Role::Member).await;

    let unknown = adam.call("PUT", &config, None).await;
    assert_eq!(
        unknown.status,
        StatusCode::BAD_REQUEST,
        "{}",
        unknown.text()
    );
    assert!(unknown.text().contains("GH_TOKEN"), "{}", unknown.text());
    for path in [&token, &config, &token] {
        let attached = adam.call("PUT", path, None).await;
        assert_eq!(
            attached.status,
            StatusCode::NO_CONTENT,
            "{}",
            attached.text()
        );
    }
    let conflict = adam.call("PUT", &other, None).await;
    assert_eq!(conflict.status, StatusCode::CONFLICT, "{}", conflict.text());
    let theirs = eve.call("PUT", &token, None).await;
    assert_eq!(theirs.status, StatusCode::NOT_FOUND);
    let missing = adam
        .call(
            "PUT",
            "/api/sandboxes/missing/setup/00000000-0000-0000-0000-000000000000",
            None,
        )
        .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);

    let listed = adam.get("/api/sandboxes/main/setup").await.json();
    let attached: Vec<(&Value, &Value)> = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|attached| {
            (
                &attached["entry"]["target"],
                &attached["entry"]["sandboxes"],
            )
        })
        .collect();
    assert_eq!(
        attached,
        [
            (
                &json!("GH_TOKEN"),
                &json!([{ "sandbox": "main", "overridden": false, "unresolved": [] }])
            ),
            (
                &json!("tool/config.toml"),
                &json!([{ "sandbox": "main", "overridden": false, "unresolved": [] }])
            )
        ]
    );
    assert_eq!(listed[1]["entry"]["references"], json!(["GH_TOKEN"]));

    let in_use = adam.call("DELETE", &token, None).await;
    assert_eq!(in_use.status, StatusCode::BAD_REQUEST, "{}", in_use.text());
    assert!(
        in_use.text().contains("~/tool/config.toml") && in_use.text().contains("GH_TOKEN"),
        "{}",
        in_use.text()
    );
    let restart = adam.call("POST", "/api/sandboxes/main/restart", None).await;
    assert_eq!(restart.status, StatusCode::NO_CONTENT, "{}", restart.text());
    for path in [&config, &token] {
        let detached = adam.call("DELETE", path, None).await;
        assert_eq!(
            detached.status,
            StatusCode::NO_CONTENT,
            "{}",
            detached.text()
        );
    }
    let again = adam.call("DELETE", &token, None).await;
    assert_eq!(again.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn an_entry_changes_in_place_and_a_template_previews_with_secrets_masked() {
    let corral = Corral::new();
    let (mut adam, [token, _, config]) = three_entries(&corral).await;
    for path in [&token, &config] {
        adam.call("PUT", path, None).await;
    }
    let id = token.rsplit('/').next().unwrap().to_owned();
    let preview = adam.get(&format!("{config}/preview")).await;
    assert_eq!(preview.status, StatusCode::OK, "{}", preview.text());
    assert_eq!(
        preview.json(),
        json!({ "text": "token = \"••••••••\"", "unresolved": [] })
    );

    let change = |target: &str, secret: bool| {
        json!({
            "kind": "variable",
            "target": target,
            "secret": secret,
            "template": false,
            "executable": false,
            "applyToNew": false,
        })
    };
    let renamed = adam
        .call(
            "PUT",
            &format!("/api/setup/{id}"),
            Some(change("GITHUB_TOKEN", true)),
        )
        .await;
    assert_eq!(
        renamed.status,
        StatusCode::BAD_REQUEST,
        "{}",
        renamed.text()
    );
    assert!(renamed.text().contains("GH_TOKEN"), "{}", renamed.text());
    let plain = adam
        .call(
            "PUT",
            &format!("/api/setup/{id}"),
            Some(change("GH_TOKEN", false)),
        )
        .await;
    assert_eq!(plain.status, StatusCode::OK, "{}", plain.text());
    assert_eq!(
        shown(&plain.json(), &["shown", "text"]),
        [json!("text"), json!("ghp_everyone")]
    );
    assert_eq!(plain.json()["sandboxes"][0]["sandbox"], "main");
    assert_eq!(
        adam.get(&format!("{config}/preview")).await.json()["text"],
        "token = \"ghp_everyone\""
    );
}

#[tokio::test]
async fn an_override_gives_one_sandbox_its_own_value() {
    let corral = Corral::new();
    let (mut adam, [token, _, _]) = three_entries(&corral).await;
    adam.call("PUT", &token, None).await;
    let overridden = adam
        .call(
            "PUT",
            &format!("{token}/override"),
            Some(json!({ "text": "ghp_main_only" })),
        )
        .await;
    assert_eq!(
        overridden.status,
        StatusCode::NO_CONTENT,
        "{}",
        overridden.text()
    );
    let listed = adam.get("/api/sandboxes/main/setup").await;
    assert!(
        !listed.text().contains("ghp_main_only"),
        "{}",
        listed.text()
    );
    let overriding = &listed.json()[0]["override"];
    assert_eq!(
        (&overriding["shown"], &overriding["length"]),
        (&json!("masked"), &json!(13))
    );

    adam.call("POST", "/api/sandboxes/main/restart", None).await;
    let owner = corral
        .world
        .users
        .by_handle(&Handle::parse("adam").unwrap())
        .await
        .unwrap()
        .unwrap();
    let main = corral
        .world
        .sandboxes
        .list(&owner)
        .await
        .unwrap()
        .remove(0)
        .sandbox;
    assert_eq!(
        exposed(&corral.world.engine.env_of(&main.id).unwrap()),
        ["GH_TOKEN=ghp_main_only"]
    );

    let cleared = adam
        .call("DELETE", &format!("{token}/override"), None)
        .await;
    assert_eq!(cleared.status, StatusCode::NO_CONTENT);
    assert_eq!(
        adam.get("/api/sandboxes/main/setup").await.json()[0]["override"],
        Value::Null
    );
}

#[tokio::test]
async fn member_calling_admin_routes_is_403() {
    let corral = Corral::new();
    let mut adam = corral.signed_in("adam", Role::Member).await;
    for (method, uri, body) in ADMIN_ROUTES {
        let body = body.map(|body| serde_json::from_str(body).unwrap());
        let reply = adam.call(method, uri, body).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "{method} {uri}");
    }
    assert_eq!(adam.get("/api/me").await.status, StatusCode::OK);
}

#[tokio::test]
async fn admins_list_users_and_all_sandboxes() {
    let corral = Corral::new();
    let mut root = corral.signed_in("root", Role::Admin).await;
    let mut adam = corral.signed_in("adam", Role::Member).await;
    adam.post("/api/sandboxes", json!({ "name": "main" })).await;
    root.post("/api/sandboxes", json!({ "name": "main" })).await;
    let picture = "https://idp.example.com/adam.png";
    let login = json!({
        "subject": "adam", "username": "adam", "role": "member", "picture": picture,
    });
    adam.post("/auth/test/login", login).await;

    let sandboxes = root.get("/api/admin/sandboxes").await.json();
    let owners: Vec<_> = sandboxes
        .as_array()
        .unwrap()
        .iter()
        .map(|sandbox| (sandbox["owner"].clone(), sandbox["ssh"]["username"].clone()))
        .collect();
    assert_eq!(
        owners,
        [
            (
                json!({ "handle": "adam", "picture": picture }),
                json!("main.adam")
            ),
            (
                json!({ "handle": "root", "picture": null }),
                json!("main.root")
            ),
        ]
    );
    let users = root.get("/api/admin/users").await.json();
    let handles: Vec<&Value> = users
        .as_array()
        .unwrap()
        .iter()
        .map(|u| &u["handle"])
        .collect();
    assert_eq!(handles, ["root", "adam"]);
    assert_eq!(users[1]["role"], "member");
    assert_eq!(users[1]["status"], "active");
}

#[tokio::test]
async fn admins_see_the_subject_behind_a_suffixed_handle() {
    let corral = Corral::new();
    let mut root = corral.signed_in("root", Role::Admin).await;
    let login = json!({ "subject": "s2", "username": "root", "role": "member" });
    let reply = corral.browser().post("/auth/test/login", login).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());

    let users = root.get("/api/admin/users").await.json();
    let shown: Vec<_> = users
        .as_array()
        .unwrap()
        .iter()
        .map(|user| {
            (
                user["handle"].clone(),
                user["provider"].clone(),
                user["subject"].clone(),
            )
        })
        .collect();
    assert_eq!(
        shown,
        [
            (json!("root"), json!(PROVIDER), json!("root")),
            (json!("root-2"), json!(PROVIDER), json!("s2")),
        ]
    );
}

#[tokio::test]
async fn admins_see_what_runs_any_sandbox() {
    let corral = Corral::new();
    let mut root = corral.signed_in("root", Role::Admin).await;
    let mut adam = corral.signed_in("adam", Role::Member).await;
    adam.post("/api/sandboxes", json!({ "name": "main" })).await;
    let calls = corral.world.engine.calls();
    let Some(EngineCall::Create { id, .. }) = calls.first() else {
        panic!("{calls:?}");
    };
    let engine = &corral.world.engine;
    engine.record_event(id, EventKind::Normal, "Scheduled", "on node-1");
    engine.record_event(id, EventKind::Warning, "BackOff", "restarting");

    let details = root.get("/api/admin/sandboxes/adam/main").await;
    assert_eq!(details.status, StatusCode::OK, "{}", details.text());
    let details = details.json();
    assert_eq!(
        details["sandbox"],
        root.get("/api/admin/sandboxes").await.json()[0]
    );
    assert_eq!(
        details["facts"],
        json!([
            { "label": "Sandbox", "value": id.0 },
            { "label": "Pod", "value": id.0 },
        ])
    );
    let events = details["events"].as_array().unwrap();
    let summary: Vec<(&Value, &Value, &Value, &Value)> = events
        .iter()
        .map(|event| {
            (
                &event["kind"],
                &event["object"],
                &event["reason"],
                &event["message"],
            )
        })
        .collect();
    let object = json!(format!("Sandbox {}", id.0));
    assert_eq!(
        summary,
        [
            (
                &json!("warning"),
                &object,
                &json!("BackOff"),
                &json!("restarting")
            ),
            (
                &json!("normal"),
                &object,
                &json!("Scheduled"),
                &json!("on node-1")
            ),
        ]
    );
    let at = events[0]["at"].as_str().unwrap();
    assert!(DateTime::parse_from_rfc3339(at).is_ok(), "{at}");

    adam.post("/api/sandboxes/main/stop", json!({})).await;
    let asleep = root.get("/api/admin/sandboxes/adam/main").await.json();
    assert_eq!(asleep["sandbox"]["state"], "sleeping");
    assert_eq!(asleep["facts"][1], json!({ "label": "Pod", "value": null }));

    for missing in [
        "/api/admin/sandboxes/adam/spare",
        "/api/admin/sandboxes/root/main",
        "/api/admin/sandboxes/nobody/main",
    ] {
        assert_eq!(
            root.get(missing).await.status,
            StatusCode::NOT_FOUND,
            "{missing}"
        );
    }
    assert_eq!(
        root.get("/api/admin/sandboxes/adam/Main").await.status,
        StatusCode::BAD_REQUEST
    );
}

fn user_id(users: &Value, handle: &str) -> String {
    users
        .as_array()
        .unwrap()
        .iter()
        .find(|user| user["handle"] == handle)
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

async fn owning_everything(corral: &Corral, handle: &str) -> Browser {
    let mut browser = corral.signed_in(handle, Role::Member).await;
    let created = browser
        .post("/api/sandboxes", json!({ "name": "main" }))
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.text());
    let key = browser
        .post(
            "/api/keys",
            json!({ "label": "laptop", "publicKey": ADAM_KEY }),
        )
        .await;
    assert_eq!(key.status, StatusCode::CREATED, "{}", key.text());
    let entry = browser
        .post("/api/setup", variable("GH_TOKEN", "gh-secret", true))
        .await;
    assert_eq!(entry.status, StatusCode::CREATED, "{}", entry.text());
    let defaults = browser
        .call(
            "PUT",
            "/api/me/sandbox-defaults",
            Some(json!({ "image": "base", "size": "large" })),
        )
        .await;
    assert_eq!(defaults.status, StatusCode::NO_CONTENT);
    browser
}

async fn owns_nothing_after_signing_up_again(corral: &Corral, handle: &str) {
    assert!(
        corral
            .world
            .provider
            .forgotten()
            .contains(&handle.to_owned())
    );
    let mut again = corral.signed_in(handle, Role::Member).await;
    for listing in ["/api/sandboxes", "/api/keys", "/api/setup"] {
        assert_eq!(again.get(listing).await.json(), json!([]), "{listing}");
    }
    let catalog = again.get("/api/catalog").await.json();
    assert_eq!(
        (&catalog["defaultImage"], &catalog["defaultSize"]),
        (&json!("base"), &json!("small"))
    );
}

#[tokio::test]
async fn admins_delete_users_with_everything_they_own() {
    let corral = Corral::new();
    let mut root = corral.signed_in("root", Role::Admin).await;
    let mut adam = owning_everything(&corral, "adam").await;
    let adam_id = user_id(&root.get("/api/admin/users").await.json(), "adam");

    let deleted = root
        .call("DELETE", &format!("/api/admin/users/{adam_id}"), None)
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT, "{}", deleted.text());
    assert_eq!(adam.get("/api/me").await.status, StatusCode::FORBIDDEN);
    let users = root.get("/api/admin/users").await.json();
    assert!(!users.to_string().contains("adam"), "{users}");
    assert_eq!(root.get("/api/admin/sandboxes").await.json(), json!([]));
    let engine_deletes = corral
        .world
        .engine
        .calls()
        .into_iter()
        .filter(|call| matches!(call, EngineCall::Delete(_)))
        .count();
    assert_eq!(engine_deletes, 1);
    assert_eq!(
        root.call("DELETE", &format!("/api/admin/users/{adam_id}"), None)
            .await
            .status,
        StatusCode::NOT_FOUND
    );

    owns_nothing_after_signing_up_again(&corral, "adam").await;
}

#[tokio::test]
async fn users_delete_themselves() {
    let corral = Corral::new();
    let mut root = corral.signed_in("root", Role::Admin).await;
    let mut adam = owning_everything(&corral, "adam").await;

    let deleted = adam.call("DELETE", "/api/me", None).await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT, "{}", deleted.text());
    assert_eq!(adam.get("/api/me").await.status, StatusCode::UNAUTHORIZED);
    let users = root.get("/api/admin/users").await.json();
    assert!(!users.to_string().contains("adam"), "{users}");
    assert_eq!(root.get("/api/admin/sandboxes").await.json(), json!([]));

    owns_nothing_after_signing_up_again(&corral, "adam").await;
}

#[tokio::test]
async fn a_failed_self_deletion_is_finished_by_the_same_user() {
    let corral = Corral::new();
    let mut root = corral.signed_in("root", Role::Admin).await;
    let mut adam = owning_everything(&corral, "adam").await;
    corral.world.engine.fail_deletes_after(Some(0));
    let failed = adam.call("DELETE", "/api/me", None).await;
    assert_eq!(failed.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(adam.get("/api/me").await.status, StatusCode::FORBIDDEN);
    let users = root.get("/api/admin/users").await.json();
    assert_eq!(users[1]["status"], "deleting");

    corral.world.engine.fail_deletes_after(None);
    let finished = adam.call("DELETE", "/api/me", None).await;
    assert_eq!(
        finished.status,
        StatusCode::NO_CONTENT,
        "{}",
        finished.text()
    );
    let users = root.get("/api/admin/users").await.json();
    assert!(!users.to_string().contains("adam"), "{users}");
    owns_nothing_after_signing_up_again(&corral, "adam").await;
}

#[tokio::test]
async fn a_disabled_user_cannot_delete_themselves() {
    let corral = Corral::new();
    let mut root = corral.signed_in("root", Role::Admin).await;
    let mut adam = owning_everything(&corral, "adam").await;
    let adam_id = user_id(&root.get("/api/admin/users").await.json(), "adam");
    root.post(&format!("/api/admin/users/{adam_id}/disable"), json!({}))
        .await;

    let refused = adam.call("DELETE", "/api/me", None).await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN);
    assert_eq!(
        root.get("/api/admin/sandboxes").await.json()[0]["owner"]["handle"],
        "adam"
    );
    assert_eq!(
        corral
            .world
            .engine
            .calls()
            .iter()
            .filter(|call| matches!(call, EngineCall::Delete(_)))
            .count(),
        0
    );
    root.post(&format!("/api/admin/users/{adam_id}/enable"), json!({}))
        .await;
    for listing in ["/api/keys", "/api/setup"] {
        assert_eq!(
            adam.get(listing).await.json().as_array().unwrap().len(),
            1,
            "{listing}"
        );
    }
}

#[tokio::test]
async fn core_errors_map_to_their_status() {
    let corral = Corral::new();
    let mut adam = corral.signed_in("adam", Role::Member).await;
    let main = json!({ "name": "main" });
    adam.post("/api/sandboxes", main.clone()).await;
    let duplicate = adam.post("/api/sandboxes", main).await;
    assert_eq!(duplicate.status, StatusCode::CONFLICT);
    assert_eq!(
        duplicate.json(),
        json!({ "error": "conflict: sandbox main already exists" })
    );

    for spare in 1..MAX_SANDBOXES {
        adam.post("/api/sandboxes", json!({ "name": format!("spare{spare}") }))
            .await;
    }
    let over_limit = adam.post("/api/sandboxes", json!({ "name": "over" })).await;
    assert_eq!(over_limit.status, StatusCode::UNPROCESSABLE_ENTITY);

    for invalid in [
        json!({ "name": "Main" }),
        json!({ "name": "other", "image": "nope" }),
        json!({}),
    ] {
        let reply = adam.post("/api/sandboxes", invalid.clone()).await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{invalid}");
        assert!(reply.json()["error"].is_string());
    }
    let malformed = adam
        .send(
            Request::post("/api/sandboxes")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{not json"))
                .unwrap(),
        )
        .await;
    assert_eq!(malformed.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        adam.get("/api/sandboxes/Not_A_Name").await.status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        adam.get("/api/sandboxes/missing").await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        adam.call("DELETE", "/api/keys/not-a-uuid", None)
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn keys_are_added_listed_and_removed() {
    let corral = Corral::new();
    let mut adam = corral.signed_in("adam", Role::Member).await;
    let added = adam
        .post(
            "/api/keys",
            json!({ "label": "laptop", "publicKey": ADAM_KEY }),
        )
        .await;
    assert_eq!(added.status, StatusCode::CREATED, "{}", added.text());
    let key = added.json();
    assert_eq!(key["label"], "laptop");
    assert!(key["fingerprint"].as_str().unwrap().starts_with("SHA256:"));
    assert!(!key["publicKey"].as_str().unwrap().contains('@'));
    assert_eq!(adam.get("/api/keys").await.json(), json!([key.clone()]));

    let mut eve = corral.signed_in("eve", Role::Member).await;
    let stolen = eve
        .post("/api/keys", json!({ "label": "x", "publicKey": ADAM_KEY }))
        .await;
    assert_eq!(stolen.status, StatusCode::CONFLICT);
    let id = key["id"].as_str().unwrap();
    assert_eq!(
        eve.call("DELETE", &format!("/api/keys/{id}"), None)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        adam.call("DELETE", &format!("/api/keys/{id}"), None)
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(adam.get("/api/keys").await.json(), json!([]));
}

#[tokio::test]
async fn post_and_put_accept_only_json() {
    let corral = Corral::new();
    let mut adam = corral.browser();
    let form = adam
        .send(
            Request::post("/auth/test/login")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from("subject=adam&username=adam&role=admin"))
                .unwrap(),
        )
        .await;
    assert_eq!(form.status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert!(!form.headers.contains_key(header::SET_COOKIE));
    assert_eq!(adam.get("/api/me").await.status, StatusCode::UNAUTHORIZED);
    sign_in(&mut adam, "adam", Some(Role::Member)).await;
    adam.post("/api/sandboxes", json!({ "name": "main" })).await;

    for (method, uri, content_type) in [
        ("POST", "/api/sandboxes/main/stop", None),
        ("POST", "/api/sandboxes/main/stop", Some("text/plain")),
        (
            "POST",
            "/auth/logout",
            Some("multipart/form-data; boundary=x"),
        ),
        ("POST", "/api/setup", Some("text/plain")),
    ] {
        let mut request = Request::builder().method(method).uri(uri);
        if let Some(content_type) = content_type {
            request = request.header(header::CONTENT_TYPE, content_type);
        }
        let reply = adam
            .send(request.body(Body::from(r#"{"value":"x"}"#)).unwrap())
            .await;
        assert_eq!(
            reply.status,
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "{method} {uri} {content_type:?}"
        );
    }
    assert_eq!(
        adam.get("/api/sandboxes/main").await.json()["state"],
        "running"
    );
    assert_eq!(adam.get("/api/me").await.status, StatusCode::OK);
    let charset = adam
        .send(
            Request::post("/api/sandboxes/main/stop")
                .header(header::CONTENT_TYPE, "Application/JSON; charset=utf-8")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(charset.status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn internal_errors_have_a_generic_body() {
    let corral = Options {
        secrets: Some(Arc::new(BrokenVault)),
        ..Options::default()
    }
    .build();
    let mut adam = corral.signed_in("adam", Role::Member).await;
    let reply = adam
        .post("/api/setup", variable("GH_TOKEN", "x", true))
        .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(reply.json(), json!({ "error": "internal error" }));
}
