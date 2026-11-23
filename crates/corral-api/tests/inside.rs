use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use corral_api::router;
use corral_api::testing::{Browser, Reply, api, trusting_provider};
use corral_core::app::SANDBOX_FILE;
use corral_core::domain::{Handle, HomePath, Role, Sandbox, SandboxStatus};
use corral_core::fakes::{EngineCall, PROVIDER, World, sandbox_name};
use corral_core::ports::{SandboxEngine, UserRepo};
use secrecy::ExposeSecret;
use serde_json::{Value, json};

struct Corral {
    app: Router,
    world: World,
}

impl Corral {
    fn new() -> Self {
        let world = World::new();
        Self {
            app: router(api(&world), vec![trusting_provider(PROVIDER)]),
            world,
        }
    }

    async fn member(&self, handle: &str) -> Browser {
        let mut browser = Browser::new(&self.app, "192.0.2.10:50000");
        let login = json!({ "subject": handle, "username": handle, "role": Role::Member });
        let reply = browser
            .post(&format!("/auth/{PROVIDER}/login"), login)
            .await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
        browser
    }

    /// The token the sandbox's last boot wrote into its home.
    async fn token(&self, handle: &str, name: &str) -> String {
        let id = self.sandbox(handle, name).await.id;
        let files = self.world.engine.files_of(&id).unwrap();
        let file = &files[&HomePath::parse(SANDBOX_FILE).unwrap()];
        let contents: Value = serde_json::from_slice(file.content.expose_secret()).unwrap();
        contents["token"].as_str().unwrap().to_owned()
    }

    async fn sandbox(&self, handle: &str, name: &str) -> Sandbox {
        let owner = self
            .world
            .users
            .by_handle(&Handle::parse(handle).unwrap())
            .await
            .unwrap()
            .unwrap();
        self.world
            .sandboxes
            .get(&owner, &sandbox_name(name))
            .await
            .unwrap()
            .sandbox
    }

    async fn status(&self, handle: &str, name: &str) -> SandboxStatus {
        let id = self.sandbox(handle, name).await.id;
        self.world.engine.status(&id).await.unwrap()
    }

    /// A request from inside a sandbox, with `token` as its bearer.
    async fn inside(&self, method: &str, path: &str, token: Option<&str>) -> Reply {
        let mut request = Request::builder().method(method).uri(path);
        if method == "POST" {
            request = request.header(header::CONTENT_TYPE, "application/json");
        }
        if let Some(token) = token {
            request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        let mut browser = Browser::new(&self.app, "198.51.100.7:40000");
        browser.send(request.body(Body::empty()).unwrap()).await
    }

    /// Waits for the work an accepted request runs on its own.
    async fn settled(&self, wanted: usize) {
        for _ in 0..100 {
            if starts(&self.world) >= wanted {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("the accepted request never reached the engine");
    }
}

fn starts(world: &World) -> usize {
    world
        .engine
        .calls()
        .iter()
        .filter(|call| matches!(call, EngineCall::Start { .. }))
        .count()
}

#[tokio::test]
async fn a_sandbox_reads_its_own_status_with_its_token() {
    let corral = Corral::new();
    let mut adam = corral.member("adam").await;
    let mut eve = corral.member("eve").await;
    adam.post("/api/sandboxes", json!({ "name": "main" })).await;
    eve.post("/api/sandboxes", json!({ "name": "main" })).await;
    let (adams, eves) = (
        corral.token("adam", "main").await,
        corral.token("eve", "main").await,
    );

    let status = corral.inside("GET", "/api/sandbox", Some(&adams)).await;
    assert_eq!(status.status, StatusCode::OK, "{}", status.text());
    assert_eq!(status.json(), adam.get("/api/sandboxes/main").await.json());
    let theirs = corral.inside("GET", "/api/sandbox", Some(&eves)).await;
    assert_eq!(theirs.json()["ssh"]["username"], "main.eve");
}

#[tokio::test]
async fn only_a_running_sandboxs_current_token_gets_in() {
    let corral = Corral::new();
    let mut adam = corral.member("adam").await;
    adam.post("/api/sandboxes", json!({ "name": "main" })).await;
    let token = corral.token("adam", "main").await;
    let refused = json!({ "error": "not the token of a running sandbox" });

    let bare = format!("Bearer {token}");
    for presented in [None, Some(""), Some("corral_sbx_nope"), Some(bare.as_str())] {
        for (method, path) in [
            ("GET", "/api/sandbox"),
            ("POST", "/api/sandbox/restart"),
            ("POST", "/api/sandbox/stop"),
        ] {
            let reply = corral.inside(method, path, presented).await;
            assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{method} {path}");
            assert_eq!(reply.json(), refused);
        }
    }
    let signed_in = adam.get("/api/sandbox").await;
    assert_eq!(
        signed_in.status,
        StatusCode::UNAUTHORIZED,
        "a session is not a token"
    );
    let as_session = corral.inside("GET", "/api/sandboxes", Some(&token)).await;
    assert_eq!(
        as_session.status,
        StatusCode::UNAUTHORIZED,
        "a token is not a session"
    );

    adam.call("POST", "/api/sandboxes/main/stop", None).await;
    let stopped = corral.inside("GET", "/api/sandbox", Some(&token)).await;
    assert_eq!(stopped.status, StatusCode::UNAUTHORIZED);
    adam.call("POST", "/api/sandboxes/main/wake", None).await;
    let old = corral.inside("GET", "/api/sandbox", Some(&token)).await;
    assert_eq!(
        old.status,
        StatusCode::UNAUTHORIZED,
        "a stopped sandbox's old token stays revoked"
    );
    let current = corral.token("adam", "main").await;
    let reply = corral.inside("GET", "/api/sandbox", Some(&current)).await;
    assert_eq!(reply.status, StatusCode::OK);
}

#[tokio::test]
async fn restart_and_stop_are_accepted_then_run() {
    let corral = Corral::new();
    let mut adam = corral.member("adam").await;
    let mut eve = corral.member("eve").await;
    adam.post("/api/sandboxes", json!({ "name": "main" })).await;
    eve.post("/api/sandboxes", json!({ "name": "main" })).await;
    let token = corral.token("adam", "main").await;
    let before = starts(&corral.world);

    let restart = corral
        .inside("POST", "/api/sandbox/restart", Some(&token))
        .await;
    assert_eq!(restart.status, StatusCode::ACCEPTED, "{}", restart.text());
    assert_eq!(restart.text(), "");
    corral.settled(before + 1).await;
    assert_eq!(corral.status("adam", "main").await, SandboxStatus::Running);
    let rotated = corral.token("adam", "main").await;
    assert_ne!(rotated, token);
    let old = corral
        .inside("POST", "/api/sandbox/stop", Some(&token))
        .await;
    assert_eq!(old.status, StatusCode::UNAUTHORIZED);

    let stop = corral
        .inside("POST", "/api/sandbox/stop", Some(&rotated))
        .await;
    assert_eq!(stop.status, StatusCode::ACCEPTED);
    for _ in 0..100 {
        if corral.status("adam", "main").await == SandboxStatus::Sleeping {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(corral.status("adam", "main").await, SandboxStatus::Sleeping);
    assert_eq!(
        corral.status("eve", "main").await,
        SandboxStatus::Running,
        "a token reaches only its own sandbox"
    );
    let after = corral.inside("GET", "/api/sandbox", Some(&rotated)).await;
    assert_eq!(after.status, StatusCode::UNAUTHORIZED);
}
