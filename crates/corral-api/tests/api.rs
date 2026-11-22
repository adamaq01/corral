use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{HeaderMap, Request, StatusCode, header};
use cookie::{Cookie, SameSite};
use corral_api::{Api, Key, Providers, SshEndpoint, router};
use corral_core::app::Credentials;
use corral_core::domain::{CredentialName, Handle, Role, UserId};
use corral_core::fakes::{ADAM_KEY, FakeEngine, MAX_SANDBOXES, MemoryLocalAccountRepo, World};
use corral_core::ports::{
    AccountAdmin, Answer, Challenge, Env, Identity, IdentityProvider, Method, PendingLogin,
    SecretStore,
};
use corral_core::{Error, Result};
use corral_identity_local::LocalProvider;
use secrecy::SecretString;
use serde_json::{Value, json};
use tower::ServiceExt;

const PASSWORD: &str = "correct horse battery";
const IDP_URL: &str = "https://idp.example.com/authorize?state=public-state";

struct StubOidc {
    role: Option<Role>,
}

#[async_trait]
impl IdentityProvider for StubOidc {
    fn id(&self) -> &'static str {
        "oidc"
    }

    fn method(&self) -> Method {
        Method::Redirect
    }

    async fn challenge(&self) -> Result<Challenge> {
        Ok(Challenge::Redirect {
            url: IDP_URL.to_owned(),
            pending: PendingLogin("private-verifier".to_owned()),
        })
    }

    async fn verify(&self, answer: Answer) -> Result<Identity> {
        let Answer::Redirect { pending, params } = answer else {
            return Err(Error::Invalid("redirect only".to_owned()));
        };
        let expected = [("code", "code-1"), ("state", "public-state")]
            .map(|(name, value)| (name.to_owned(), value.to_owned()));
        if pending.0 != "private-verifier" || params != BTreeMap::from(expected) {
            return Err(Error::Forbidden);
        }
        Ok(Identity {
            subject: "oidc:sub-1".to_owned(),
            username: "Adam".to_owned(),
            email: Some("adam@example.com".to_owned()),
            role: self.role,
        })
    }
}

struct CountingLocal {
    local: LocalProvider,
    verifications: AtomicUsize,
}

#[async_trait]
impl IdentityProvider for CountingLocal {
    fn id(&self) -> &str {
        self.local.id()
    }

    fn method(&self) -> Method {
        self.local.method()
    }

    async fn challenge(&self) -> Result<Challenge> {
        self.local.challenge().await
    }

    async fn verify(&self, answer: Answer) -> Result<Identity> {
        self.verifications.fetch_add(1, Ordering::SeqCst);
        self.local.verify(answer).await
    }

    fn accounts(&self) -> Option<&dyn AccountAdmin> {
        self.local.accounts()
    }
}

struct BrokenVault;

#[async_trait]
impl SecretStore for BrokenVault {
    async fn set(&self, _: UserId, _: &CredentialName, _: SecretString) -> Result<()> {
        Err(Error::internal(
            "vault",
            "vault at 10.9.8.7 refused token s.internal",
        ))
    }

    async fn remove(&self, _: UserId, _: &CredentialName) -> Result<bool> {
        Ok(false)
    }

    async fn names(&self, _: UserId) -> Result<Vec<CredentialName>> {
        Ok(Vec::new())
    }

    async fn load(&self, _: UserId) -> Result<Env> {
        Ok(Env::new())
    }
}

struct Setup {
    oidc: Option<StubOidc>,
    local: bool,
    secure: bool,
    trust_forwarded_for: bool,
    secrets: Option<Arc<dyn SecretStore>>,
}

impl Default for Setup {
    fn default() -> Self {
        Self {
            oidc: None,
            local: true,
            secure: false,
            trust_forwarded_for: false,
            secrets: None,
        }
    }
}

struct Corral {
    app: Router,
    local: Arc<CountingLocal>,
    engine: Arc<FakeEngine>,
}

impl Setup {
    fn build(self) -> Corral {
        let world = World::new();
        let local = Arc::new(CountingLocal {
            local: LocalProvider::new(
                Arc::new(MemoryLocalAccountRepo::default()),
                world.users.clone(),
            )
            .unwrap(),
            verifications: AtomicUsize::new(0),
        });
        let mut providers: Vec<Arc<dyn IdentityProvider>> = Vec::new();
        if self.local {
            providers.push(local.clone());
        }
        if let Some(oidc) = self.oidc {
            providers.push(Arc::new(oidc));
        }
        let api = Api {
            accounts: world.accounts.clone(),
            keys: world.keys.clone(),
            credentials: self
                .secrets
                .map_or_else(|| world.credentials.clone(), Credentials::new),
            sandboxes: world.sandboxes.clone(),
            providers: Providers::new(providers),
            ssh: SshEndpoint {
                host: "corral.example.com".to_owned(),
                port: 48291,
            },
            session_key: Key::generate(),
            secure_cookies: self.secure,
            trust_forwarded_for: self.trust_forwarded_for,
        };
        Corral {
            app: router(api),
            local,
            engine: world.engine,
        }
    }
}

impl Corral {
    fn new() -> Self {
        Setup::default().build()
    }

    async fn account(&self, username: &str, role: Role) {
        self.local
            .local
            .add(
                &Handle::parse(username).unwrap(),
                SecretString::from(PASSWORD),
                role,
            )
            .await
            .unwrap();
    }

    fn verifications(&self) -> usize {
        self.local.verifications.load(Ordering::SeqCst)
    }

    fn browser(&self) -> Browser {
        self.browser_at("192.0.2.10:50000")
    }

    fn browser_at(&self, peer: &str) -> Browser {
        Browser {
            app: self.app.clone(),
            peer: peer.parse().unwrap(),
            cookies: BTreeMap::new(),
        }
    }

    async fn signed_in(&self, username: &str, role: Role) -> Browser {
        self.account(username, role).await;
        let mut browser = self.browser();
        let reply = browser.sign_in(username, PASSWORD).await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
        browser
    }
}

struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    body: Vec<u8>,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|_| panic!("not JSON: {}", self.text()))
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    fn set_cookie(&self, name: &str) -> Cookie<'static> {
        self.headers
            .get_all(header::SET_COOKIE)
            .iter()
            .map(|value| Cookie::parse(value.to_str().unwrap().to_owned()).unwrap())
            .find(|cookie| cookie.name() == name)
            .unwrap_or_else(|| panic!("no {name} cookie in {:?}", self.headers))
    }
}

struct Browser {
    app: Router,
    peer: SocketAddr,
    cookies: BTreeMap<String, String>,
}

impl Browser {
    async fn send(&mut self, mut request: Request<Body>) -> Reply {
        if !self.cookies.is_empty() {
            let line = self
                .cookies
                .iter()
                .map(|(name, value)| format!("{name}={value}"))
                .collect::<Vec<_>>()
                .join("; ");
            request
                .headers_mut()
                .insert(header::COOKIE, line.parse().unwrap());
        }
        request.extensions_mut().insert(ConnectInfo(self.peer));
        let response = self.app.clone().oneshot(request).await.unwrap();
        let (parts, body) = response.into_parts();
        for value in parts.headers.get_all(header::SET_COOKIE) {
            let cookie = Cookie::parse(value.to_str().unwrap().to_owned()).unwrap();
            if cookie.value().is_empty() {
                self.cookies.remove(cookie.name());
            } else {
                self.cookies
                    .insert(cookie.name().to_owned(), cookie.value().to_owned());
            }
        }
        Reply {
            status: parts.status,
            headers: parts.headers,
            body: axum::body::to_bytes(body, usize::MAX)
                .await
                .unwrap()
                .to_vec(),
        }
    }

    async fn call(&mut self, method: &str, uri: &str, body: Option<Value>) -> Reply {
        let mut request = Request::builder().method(method).uri(uri);
        if matches!(method, "POST" | "PUT") {
            request = request.header(header::CONTENT_TYPE, "application/json");
        }
        let body = body.map_or_else(Body::empty, |body| Body::from(body.to_string()));
        self.send(request.body(body).unwrap()).await
    }

    async fn get(&mut self, uri: &str) -> Reply {
        self.call("GET", uri, None).await
    }

    async fn post(&mut self, uri: &str, body: Value) -> Reply {
        self.call("POST", uri, Some(body)).await
    }

    async fn sign_in(&mut self, username: &str, password: &str) -> Reply {
        self.sign_in_via(username, password, None).await
    }

    async fn sign_in_via(
        &mut self,
        username: &str,
        password: &str,
        forwarded_for: Option<&str>,
    ) -> Reply {
        let mut request =
            Request::post("/auth/local/finish").header(header::CONTENT_TYPE, "application/json");
        if let Some(forwarded_for) = forwarded_for {
            request = request.header("x-forwarded-for", forwarded_for);
        }
        let body = json!({ "username": username, "password": password }).to_string();
        self.send(request.body(Body::from(body)).unwrap()).await
    }
}

#[tokio::test]
async fn local_login_sets_a_session_and_me_returns_the_handle() {
    let corral = Corral::new();
    corral.account("adam", Role::Member).await;
    let mut browser = corral.browser();
    assert_eq!(
        browser.get("/api/me").await.status,
        StatusCode::UNAUTHORIZED
    );

    let reply = browser.sign_in("adam", PASSWORD).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.json(),
        json!({ "handle": "adam", "email": null, "role": "member" })
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
async fn sixth_failed_local_login_within_the_window_is_429() {
    let corral = Corral::new();
    corral.account("adam", Role::Member).await;
    corral.account("eve", Role::Member).await;
    let mut attacker = corral.browser_at("198.51.100.66:4000");
    for attempt in 1..=5 {
        let reply = attacker
            .sign_in("adam", &format!("guess number {attempt}"))
            .await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "attempt {attempt}");
    }
    let verified = corral.verifications();
    for password in ["guess number 6", PASSWORD] {
        let reply = attacker.sign_in("adam", password).await;
        assert_eq!(reply.status, StatusCode::TOO_MANY_REQUESTS, "{password}");
        assert!(reply.json()["error"].is_string());
    }
    assert_eq!(
        attacker.get("/api/me").await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(corral.verifications(), verified);

    let mut adam = corral.browser_at("192.0.2.1:5000");
    assert_eq!(
        adam.sign_in("adam", PASSWORD).await.status,
        StatusCode::TOO_MANY_REQUESTS
    );
    let mut eve = corral.browser_at("192.0.2.2:5000");
    assert_eq!(eve.sign_in("eve", PASSWORD).await.status, StatusCode::OK);
}

#[tokio::test]
async fn successful_local_logins_are_never_limited() {
    let corral = Corral::new();
    corral.account("adam", Role::Member).await;
    let mut browser = corral.browser();
    for attempt in 1..=10 {
        let reply = browser.sign_in("adam", PASSWORD).await;
        assert_eq!(reply.status, StatusCode::OK, "attempt {attempt}");
    }
}

#[tokio::test]
async fn a_successful_login_does_not_reset_the_failures() {
    let corral = Corral::new();
    corral.account("adam", Role::Member).await;
    corral.account("eve", Role::Member).await;
    let mut attacker = corral.browser_at("198.51.100.66:4000");
    for attempt in 1..=4 {
        let reply = attacker.sign_in("adam", &format!("guess {attempt}")).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "attempt {attempt}");
    }
    assert_eq!(
        attacker.sign_in("eve", PASSWORD).await.status,
        StatusCode::OK
    );
    assert_eq!(
        attacker.sign_in("adam", "guess 5").await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        attacker.sign_in("eve", PASSWORD).await.status,
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn trusted_forwarded_for_addresses_are_counted_separately() {
    let corral = Setup {
        trust_forwarded_for: true,
        ..Setup::default()
    }
    .build();
    corral.account("adam", Role::Member).await;
    let mut proxy = corral.browser_at("10.0.0.2:40000");
    for i in 1..=5 {
        let reply = proxy
            .sign_in_via(&format!("user-{i}"), PASSWORD, Some("203.0.113.7"))
            .await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "{i}");
    }
    for spoofed in ["203.0.113.7", "198.51.100.9, 203.0.113.7"] {
        let reply = proxy.sign_in_via("adam", PASSWORD, Some(spoofed)).await;
        assert_eq!(reply.status, StatusCode::TOO_MANY_REQUESTS, "{spoofed}");
    }
    let reply = proxy
        .sign_in_via("adam", PASSWORD, Some("203.0.113.7, 198.51.100.9"))
        .await;
    assert_eq!(reply.status, StatusCode::OK);
}

#[tokio::test]
async fn untrusted_forwarded_for_is_ignored() {
    let corral = Corral::new();
    corral.account("adam", Role::Member).await;
    let mut client = corral.browser_at("10.0.0.2:40000");
    for i in 1..=5 {
        let forwarded = format!("203.0.113.{i}");
        let reply = client
            .sign_in_via(&format!("user-{i}"), PASSWORD, Some(&forwarded))
            .await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "{i}");
    }
    let reply = client
        .sign_in_via("adam", PASSWORD, Some("198.51.100.9"))
        .await;
    assert_eq!(reply.status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn cookies_lack_secure_when_base_url_is_http() {
    for secure in [false, true] {
        let corral = Setup {
            secure,
            oidc: Some(StubOidc {
                role: Some(Role::Member),
            }),
            ..Setup::default()
        }
        .build();
        corral.account("adam", Role::Member).await;
        let mut browser = corral.browser();
        let session = browser
            .sign_in("adam", PASSWORD)
            .await
            .set_cookie("corral_session");
        let login = browser
            .get("/auth/oidc/start")
            .await
            .set_cookie("corral_login");
        for cookie in [session, login] {
            assert_eq!(cookie.secure(), secure.then_some(true), "{cookie}");
            assert_eq!(cookie.http_only(), Some(true), "{cookie}");
            assert_eq!(cookie.same_site(), Some(SameSite::Lax), "{cookie}");
            assert_eq!(cookie.path(), Some("/"), "{cookie}");
        }
    }
}

#[tokio::test]
async fn providers_follow_the_configuration() {
    let both = Setup {
        oidc: Some(StubOidc {
            role: Some(Role::Member),
        }),
        ..Setup::default()
    }
    .build();
    assert_eq!(
        both.browser().get("/auth/providers").await.json(),
        json!([
            { "id": "local", "method": "password" },
            { "id": "oidc", "method": "redirect" },
        ])
    );

    let oidc_only = Setup {
        oidc: Some(StubOidc {
            role: Some(Role::Admin),
        }),
        local: false,
        ..Setup::default()
    }
    .build();
    let mut browser = oidc_only.browser();
    assert_eq!(
        browser.get("/auth/providers").await.json(),
        json!([{ "id": "oidc", "method": "redirect" }])
    );
    assert_eq!(
        browser.sign_in("adam", PASSWORD).await.status,
        StatusCode::NOT_FOUND
    );
    let password_to_oidc = browser
        .post(
            "/auth/oidc/finish",
            json!({ "username": "adam", "password": PASSWORD }),
        )
        .await;
    assert_eq!(password_to_oidc.status, StatusCode::BAD_REQUEST);
    browser.get("/auth/oidc/start").await;
    browser
        .get("/auth/oidc/finish?code=code-1&state=public-state")
        .await;
    assert_eq!(browser.get("/api/me").await.json()["role"], "admin");
    for (method, uri, body) in ADMIN_ROUTES
        .iter()
        .filter(|(_, uri, _)| uri.contains("local"))
    {
        let body = body.map(|body| serde_json::from_str(body).unwrap());
        let reply = browser.call(method, uri, body).await;
        assert_eq!(reply.status, StatusCode::NOT_FOUND, "{method} {uri}");
    }

    let local_only = Corral::new();
    let mut browser = local_only.browser();
    assert_eq!(
        browser.get("/auth/providers").await.json(),
        json!([{ "id": "local", "method": "password" }])
    );
    let start = browser.get("/auth/local/start").await;
    assert_eq!(start.status, StatusCode::OK);
    assert_eq!(start.json(), json!({ "id": "local", "method": "password" }));
    assert!(!start.headers.contains_key(header::SET_COOKIE));
    assert_eq!(
        browser
            .get("/auth/local/finish?code=code-1&state=public-state")
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        browser.get("/auth/oidc/start").await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        browser
            .get("/auth/oidc/finish?code=code-1&state=public-state")
            .await
            .status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn oidc_login_round_trip_starts_a_session() {
    let corral = Setup {
        oidc: Some(StubOidc {
            role: Some(Role::Member),
        }),
        ..Setup::default()
    }
    .build();
    let mut browser = corral.browser();
    let start = browser.get("/auth/oidc/start").await;
    assert_eq!(start.status, StatusCode::FOUND);
    assert_eq!(start.headers[header::LOCATION], IDP_URL);
    let login_cookie = start.set_cookie("corral_login");
    assert!(!login_cookie.value().contains("private-verifier"));
    assert_eq!(
        login_cookie.max_age(),
        Some(cookie::time::Duration::minutes(10))
    );

    let done = browser
        .get("/auth/oidc/finish?code=code-1&state=public-state")
        .await;
    assert_eq!(done.status, StatusCode::FOUND, "{}", done.text());
    assert_eq!(done.headers[header::LOCATION], "/");
    assert!(!browser.cookies.contains_key("corral_login"));
    assert_eq!(
        browser.get("/api/me").await.json(),
        json!({ "handle": "adam", "email": "adam@example.com", "role": "member" })
    );

    let replay = browser
        .get("/auth/oidc/finish?code=code-1&state=public-state")
        .await;
    assert_eq!(replay.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn oidc_callback_refusals_are_403_and_end_the_login() {
    let corral = Setup {
        oidc: Some(StubOidc { role: None }),
        ..Setup::default()
    }
    .build();
    let mut browser = corral.browser();
    browser.get("/auth/oidc/start").await;
    let wrong_state = browser
        .get("/auth/oidc/finish?code=code-1&state=forged")
        .await;
    assert_eq!(wrong_state.status, StatusCode::FORBIDDEN);
    assert!(!browser.cookies.contains_key("corral_login"));

    browser.get("/auth/oidc/start").await;
    let not_a_member = browser
        .get("/auth/oidc/finish?code=code-1&state=public-state")
        .await;
    assert_eq!(not_a_member.status, StatusCode::FORBIDDEN);
    assert_eq!(not_a_member.json(), json!({ "error": "forbidden" }));
    assert_eq!(
        browser.get("/api/me").await.status,
        StatusCode::UNAUTHORIZED
    );
}

const API_ROUTES: &[(&str, &str)] = &[
    ("GET", "/api/me"),
    ("GET", "/api/catalog"),
    ("GET", "/api/sandboxes"),
    ("POST", "/api/sandboxes"),
    ("GET", "/api/sandboxes/main"),
    ("DELETE", "/api/sandboxes/main"),
    ("POST", "/api/sandboxes/main/wake"),
    ("POST", "/api/sandboxes/main/stop"),
    ("POST", "/api/sandboxes/main/restart"),
    ("POST", "/api/sandboxes/main/default"),
    ("GET", "/api/keys"),
    ("POST", "/api/keys"),
    ("DELETE", "/api/keys/00000000-0000-0000-0000-000000000000"),
    ("GET", "/api/credentials"),
    ("PUT", "/api/credentials/GH_TOKEN"),
    ("DELETE", "/api/credentials/GH_TOKEN"),
];

const ADMIN_ROUTES: &[(&str, &str, Option<&str>)] = &[
    ("GET", "/api/admin/users", None),
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
    ("GET", "/api/admin/sandboxes", None),
    ("GET", "/api/admin/local-accounts", None),
    (
        "POST",
        "/api/admin/local-accounts",
        Some(r#"{"username":"mallory","password":"a long enough password","role":"admin"}"#),
    ),
    (
        "PUT",
        "/api/admin/local-accounts/adam/password",
        Some(r#"{"password":"a long enough password"}"#),
    ),
    ("DELETE", "/api/admin/local-accounts/adam", None),
];

#[tokio::test]
async fn unauthenticated_api_calls_are_401() {
    let corral = Corral::new();
    let routes = API_ROUTES
        .iter()
        .copied()
        .chain(ADMIN_ROUTES.iter().map(|&(method, uri, _)| (method, uri)));
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
    assert_eq!(disabled.json()["enabled"], false);
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
    let created = adam.post("/api/sandboxes", json!({ "name": "main" })).await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.text());
    let expected = json!({
        "name": "main",
        "image": "base",
        "size": "small",
        "isDefault": true,
        "state": "running",
        "ssh": { "username": "adam+main", "host": "corral.example.com", "port": 48291 },
    });
    assert_eq!(created.json(), expected);
    assert_eq!(adam.get("/api/sandboxes").await.json(), json!([expected]));
    assert_eq!(adam.get("/api/sandboxes/main").await.json(), expected);

    assert_eq!(
        adam.get("/api/catalog").await.json(),
        json!({
            "images": [{ "key": "base", "label": "BASE" }, { "key": "tools", "label": "TOOLS" }],
            "sizes": [{ "key": "small", "label": "SMALL" }, { "key": "large", "label": "LARGE" }],
            "defaultImage": "base",
            "defaultSize": "small",
            "ssh": { "host": "corral.example.com", "port": 48291 },
        })
    );
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

    corral.engine.set_waking(true);
    let waking = adam.call("POST", "/api/sandboxes/main/wake", None).await;
    assert_eq!(waking.json(), json!({ "ready": false }));
    corral.engine.set_waking(false);
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

#[tokio::test]
async fn credential_values_never_come_back() {
    let corral = Corral::new();
    let mut adam = corral.signed_in("adam", Role::Member).await;
    let mut replies = vec![
        adam.call(
            "PUT",
            "/api/credentials/GH_TOKEN",
            Some(json!({ "value": "ghp_s3cr3t-value" })),
        )
        .await,
    ];
    assert_eq!(replies[0].status, StatusCode::NO_CONTENT);
    replies.push(adam.get("/api/credentials").await);
    assert_eq!(replies[1].json(), json!([{ "name": "GH_TOKEN" }]));
    replies.push(
        adam.call(
            "PUT",
            "/api/credentials/GH_TOKEN",
            Some(json!({ "value": "ghp_s3cr3t\nvalue" })),
        )
        .await,
    );
    assert_eq!(replies[2].status, StatusCode::BAD_REQUEST);
    replies.push(adam.post("/api/sandboxes", json!({ "name": "main" })).await);
    replies.push(adam.get("/api/sandboxes").await);
    for reply in &replies {
        assert!(!reply.text().contains("s3cr3t"), "{}", reply.text());
    }

    let removed = adam.call("DELETE", "/api/credentials/GH_TOKEN", None).await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT);
    assert_eq!(adam.get("/api/credentials").await.json(), json!([]));
    let invalid = adam
        .call(
            "PUT",
            "/api/credentials/gh-token",
            Some(json!({ "value": "x" })),
        )
        .await;
    assert_eq!(invalid.status, StatusCode::BAD_REQUEST);
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
    let mut mallory = corral.browser();
    assert_eq!(
        mallory
            .sign_in("mallory", "a long enough password")
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(adam.get("/api/me").await.status, StatusCode::OK);
}

#[tokio::test]
async fn admins_manage_users_local_accounts_and_all_sandboxes() {
    let corral = Corral::new();
    let mut root = corral.signed_in("root", Role::Admin).await;
    let mut adam = corral.signed_in("adam", Role::Member).await;
    adam.post("/api/sandboxes", json!({ "name": "main" })).await;

    let sandboxes = root.get("/api/admin/sandboxes").await.json();
    assert_eq!(sandboxes[0]["owner"], "adam");
    assert_eq!(sandboxes[0]["ssh"]["username"], "adam+main");
    let users = root.get("/api/admin/users").await.json();
    let handles: Vec<&Value> = users
        .as_array()
        .unwrap()
        .iter()
        .map(|u| &u["handle"])
        .collect();
    assert_eq!(handles, ["root", "adam"]);
    assert_eq!(users[1]["role"], "member");
    assert_eq!(users[1]["enabled"], true);

    let created = root
        .post(
            "/api/admin/local-accounts",
            json!({ "username": "eve", "password": "eve's first password", "role": "member" }),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.text());
    let reset = root
        .call(
            "PUT",
            "/api/admin/local-accounts/eve/password",
            Some(json!({ "password": "eve's second password" })),
        )
        .await;
    assert_eq!(reset.status, StatusCode::NO_CONTENT);
    let mut eve = corral.browser_at("192.0.2.77:1000");
    assert_eq!(
        eve.sign_in("eve", "eve's second password").await.status,
        StatusCode::OK
    );
    let accounts = root.get("/api/admin/local-accounts").await;
    assert!(!accounts.text().contains("argon2"), "{}", accounts.text());
    let listed: Vec<Value> = accounts
        .json()
        .as_array()
        .unwrap()
        .iter()
        .map(|account| json!([account["username"], account["role"]]))
        .collect();
    assert_eq!(
        listed,
        [
            json!(["adam", "member"]),
            json!(["eve", "member"]),
            json!(["root", "admin"])
        ]
    );

    let deleted = root
        .call("DELETE", "/api/admin/local-accounts/eve", None)
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    assert_eq!(eve.get("/api/me").await.status, StatusCode::FORBIDDEN);
    let reused = root
        .post(
            "/api/admin/local-accounts",
            json!({ "username": "eve", "password": "a newcomer's password", "role": "member" }),
        )
        .await;
    assert_eq!(reused.status, StatusCode::CONFLICT);
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
        json!({ "error": "conflict: sandbox main" })
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
    corral.account("adam", Role::Member).await;
    let mut adam = corral.browser();
    let form = adam
        .send(
            Request::post("/auth/local/finish")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(format!("username=adam&password={PASSWORD}")))
                .unwrap(),
        )
        .await;
    assert_eq!(form.status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert_eq!(corral.verifications(), 0);
    adam.sign_in("adam", PASSWORD).await;
    adam.post("/api/sandboxes", json!({ "name": "main" })).await;

    for (method, uri, content_type) in [
        ("POST", "/api/sandboxes/main/stop", None),
        ("POST", "/api/sandboxes/main/stop", Some("text/plain")),
        (
            "POST",
            "/auth/logout",
            Some("multipart/form-data; boundary=x"),
        ),
        ("PUT", "/api/credentials/GH_TOKEN", Some("text/plain")),
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
    let corral = Setup {
        secrets: Some(Arc::new(BrokenVault)),
        ..Setup::default()
    }
    .build();
    let mut adam = corral.signed_in("adam", Role::Member).await;
    let reply = adam
        .call(
            "PUT",
            "/api/credentials/GH_TOKEN",
            Some(json!({ "value": "x" })),
        )
        .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(reply.json(), json!({ "error": "internal error" }));
}

#[tokio::test]
async fn a_missing_peer_address_fails_closed() {
    let corral = Corral::new();
    corral.account("adam", Role::Member).await;
    let request = Request::post("/auth/local/finish")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({ "username": "adam", "password": PASSWORD }).to_string(),
        ))
        .unwrap();
    let response = corral.app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(response.headers().get(header::SET_COOKIE).is_none());
}
