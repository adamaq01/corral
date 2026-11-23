use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode, header};
use chrono::{DateTime, Utc};
use corral_core::app::{Credentials, Limits, Sandboxes, SshAccess};
use corral_core::domain::{Handle, SandboxDefaults, SandboxId, SandboxStatus, User, UserId};
use corral_core::fakes::{
    ADAM_CERT, EngineCall, FakeEngine, MAX_SANDBOXES, MemoryKeyRepo, MemorySandboxRepo,
    MemorySecretStore, WAKE_DEADLINE, World, admin_proof, catalog, links, sandbox_name,
};
use corral_core::ports::{
    Boot, Inspection, SandboxEngine, SandboxSpec, SshTarget, Started, UserRepo,
};
use corral_core::{Error, Result};
use corral_frontdoor_containerssh::{Cidr, ContainerSshConfig, router};
use serde_json::{Value, json};
use tokio::sync::Semaphore;
use tokio::time::timeout;
use tower::ServiceExt;

use crate::common::{Fixture, Member, generate_key, public_line};

async fn send(app: &Router, method: Method, path: &str, body: String) -> (StatusCode, Value) {
    let label = format!("{method} {path}");
    let request = Request::builder()
        .method(method)
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE),
        Some(&header::HeaderValue::from_static("application/json")),
        "{label} answered {status}"
    );
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

async fn pubkey(
    app: &Router,
    username: &str,
    public_key: &str,
    remote_address: &str,
    connection_id: &str,
) -> (StatusCode, Value) {
    let body = json!({
        "remoteAddress": remote_address,
        "connectionId": connection_id,
        "clientVersion": "SSH-2.0-OpenSSH_9.6",
        "username": username,
        "publicKey": public_key,
    });
    send(app, Method::POST, "/pubkey", body.to_string()).await
}

async fn accepted(app: &Router, username: &str, public_key: &str, remote_address: &str) -> bool {
    let connection_id = format!("connection-{remote_address}");
    let (status, body) = pubkey(app, username, public_key, remote_address, &connection_id).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        json!({"success": body["success"], "authenticatedUsername": username})
    );
    body["success"].as_bool().unwrap()
}

async fn config(app: &Router, username: &str, authenticated_username: &str) -> (StatusCode, Value) {
    let body = json!({
        "remoteAddress": "1.2.3.4:50022",
        "connectionId": "c0ffee",
        "clientVersion": "SSH-2.0-OpenSSH_9.6",
        "username": username,
        "authenticatedUsername": authenticated_username,
    });
    send(app, Method::POST, "/config", body.to_string()).await
}

async fn disable(w: &Fixture, member: &Member) {
    let root = w.world.admin("root").await;
    w.world
        .accounts
        .set_enabled(&admin_proof(&root), member.user.id, false)
        .await
        .unwrap();
}

#[tokio::test]
async fn pubkey_accepts_the_owners_key() {
    let w = Fixture::new().await;
    let app = w.router();
    for username in ["adam", "main.adam", "new.adam"] {
        assert!(accepted(&app, username, &w.adam.public_key(), "1.2.3.4:50022").await);
    }
    assert!(accepted(&app, "main.eve", &w.eve.public_key(), "1.2.3.4:50022").await);
}

#[tokio::test]
async fn pubkey_accepts_a_certificate_the_owners_provider_vouches_for() {
    let w = Fixture::new().await;
    w.world.provider.vouch("adam", ADAM_CERT);
    let app = w.router();
    assert!(accepted(&app, "main.adam", ADAM_CERT, "1.2.3.4:50022").await);
    assert!(!accepted(&app, "main.eve", ADAM_CERT, "1.2.3.4:50022").await);
}

#[tokio::test]
async fn pubkey_denies_with_200_and_false() {
    let w = Fixture::new().await;
    let app = w.router();
    let adam = w.adam.public_key();
    let eve = w.eve.public_key();
    let stranger = public_line(&generate_key());
    for (username, key) in [
        ("main.adam", stranger.as_str()),
        ("main.adam", eve.as_str()),
        ("adam", eve.as_str()),
        ("main.eve", adam.as_str()),
        ("nobody", adam.as_str()),
        ("new.adam", eve.as_str()),
        ("adam+main", adam.as_str()),
        ("ADAM", adam.as_str()),
        ("main.adam", "not a key"),
    ] {
        assert!(
            !accepted(&app, username, key, "1.2.3.4:50022").await,
            "{username}"
        );
    }
    disable(&w, &w.adam).await;
    assert!(!accepted(&app, "main.adam", &adam, "1.2.3.4:50022").await);
}

#[tokio::test(start_paused = true)]
async fn pubkey_locks_out_an_address_after_repeated_failures() {
    let w = Fixture::new().await;
    let app = w.router();
    let adam = w.adam.public_key();
    let stranger = public_line(&generate_key());
    for attempt in 0..10 {
        if attempt == 9 {
            assert!(accepted(&app, "adam", &adam, "1.2.3.4:40000").await);
        }
        let address = format!("1.2.3.4:{}", 50000 + attempt);
        let (_, body) = pubkey(&app, "adam", &stranger, &address, &format!("c{attempt}")).await;
        assert_eq!(body["success"], false);
    }

    assert!(!accepted(&app, "adam", &adam, "1.2.3.4:40001").await);
    assert!(!accepted(&app, "eve", &w.eve.public_key(), "1.2.3.4:40002").await);
    assert!(accepted(&app, "adam", &adam, "5.6.7.8:40000").await);

    tokio::time::advance(Duration::from_mins(5)).await;
    assert!(accepted(&app, "adam", &adam, "1.2.3.4:40003").await);
}

#[tokio::test]
async fn pubkey_counts_a_connection_offering_many_keys_once() {
    let w = Fixture::new().await;
    let app = w.router();
    let unregistered: Vec<String> = (0..3).map(|_| public_line(&generate_key())).collect();
    for connection in 0..9 {
        let address = format!("1.2.3.4:{}", 50000 + connection);
        for key in &unregistered {
            pubkey(&app, "adam", key, &address, &format!("c{connection}")).await;
        }
    }
    assert!(accepted(&app, "adam", &w.adam.public_key(), "1.2.3.4:40000").await);
}

#[tokio::test]
async fn pubkey_locks_out_rotating_addresses_in_one_ipv6_64_together() {
    let w = Fixture::new().await;
    let app = w.router();
    let adam = w.adam.public_key();
    let stranger = public_line(&generate_key());
    for host in 1..=10 {
        let address = format!("[2001:db8:0:7::{host:x}]:50000");
        let (_, body) = pubkey(&app, "adam", &stranger, &address, &format!("c{host}")).await;
        assert_eq!(body["success"], false);
    }
    assert!(!accepted(&app, "adam", &adam, "[2001:db8:0:7:ffff::1]:40000").await);
    assert!(accepted(&app, "adam", &adam, "[2001:db8:0:8::1]:40000").await);
}

#[tokio::test]
async fn pubkey_never_locks_out_an_exempt_network_and_still_locks_out_others() {
    let w = Fixture::new().await;
    let app = w.router_with(ContainerSshConfig {
        lockout_exempt_networks: vec![Cidr::try_from("10.42.0.0/16".to_owned()).unwrap()],
    });
    let adam = w.adam.public_key();
    let stranger = public_line(&generate_key());
    for attempt in 0..30 {
        for host in ["10.42.0.7", "10.42.3.9", "10.43.0.1"] {
            let address = format!("{host}:{}", 50000 + attempt);
            let connection = format!("{host}-{attempt}");
            let (_, body) = pubkey(&app, "adam", &stranger, &address, &connection).await;
            assert_eq!(body["success"], false);
        }
    }
    assert!(accepted(&app, "adam", &adam, "10.42.0.7:40000").await);
    assert!(accepted(&app, "eve", &w.eve.public_key(), "[::ffff:10.42.3.9]:40000").await);
    assert!(!accepted(&app, "adam", &adam, "10.43.0.1:40000").await);
    assert!(accepted(&app, "adam", &adam, "1.2.3.4:40000").await);
}

#[tokio::test]
async fn config_answers_each_target_kind() {
    let w = Fixture::new().await;
    let app = w.router();
    w.world.engine.set_target(
        &w.eve.main.id,
        SshTarget::SshServer {
            host: "corral-sbx-eve".to_owned(),
            port: 22,
            username: "sandbox".to_owned(),
            host_key_fingerprint: "SHA256:abc".to_owned(),
        },
    );

    let (status, body) = config(&app, "main.adam", "main.adam").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        json!({"config": {"backend": "kubernetes", "kubernetes": {"pod": {
            "metadata": {"name": w.adam.main.id.0, "namespace": FakeEngine::NAMESPACE},
            "mode": "persistent",
            "shellCommand": ["/bin/bash", "-l"],
        }}}})
    );

    let (status, body) = config(&app, "eve", "eve").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        json!({"config": {"backend": "sshproxy", "sshproxy": {
            "server": "corral-sbx-eve",
            "port": 22,
            "username": "sandbox",
            "allowedHostKeyFingerprints": ["SHA256:abc"],
        }}})
    );
}

#[tokio::test]
async fn config_routes_the_authenticated_username() {
    let w = Fixture::new().await;
    let app = w.router();
    let (status, body) = config(&app, "main.eve", "main.adam").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["config"]["kubernetes"]["pod"]["metadata"]["name"],
        w.adam.main.id.0
    );
}

#[tokio::test]
async fn config_returns_503_while_waking() {
    let w = Fixture::new().await;
    let app = w.router();
    let main = &w.adam.main;
    w.world
        .sandboxes
        .stop(&w.adam.user, &main.name)
        .await
        .unwrap();
    w.world.engine.set_waking(true);
    let starts = w.starts(&main.id);
    assert_eq!(
        config(&app, "main.adam", "main.adam").await,
        (StatusCode::SERVICE_UNAVAILABLE, json!({}))
    );
    assert_eq!(w.starts(&main.id), starts + 1);
    assert_eq!(w.starts(&w.eve.main.id), 1);
    w.world.engine.set_waking(false);
    assert_eq!(
        config(&app, "main.adam", "main.adam").await.0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn config_refuses_unknown_disabled_or_missing_with_403() {
    let w = Fixture::new().await;
    let app = w.router();
    disable(&w, &w.eve).await;
    for username in [
        "nobody",
        "eve",
        "main.eve",
        "new.eve",
        "ADAM",
        "adam+main",
        "new_box.adam",
        "",
    ] {
        assert_eq!(
            config(&app, username, username).await,
            (StatusCode::FORBIDDEN, json!({})),
            "{username}"
        );
    }
}

#[tokio::test]
async fn config_makes_the_sandbox_a_login_names_within_the_owners_limit() {
    let w = Fixture::new().await;
    let app = w.router();
    let (status, body) = config(&app, "scratch.adam", "scratch.adam").await;
    assert_eq!(status, StatusCode::OK);
    let scratch = w
        .world
        .sandboxes
        .get(&w.adam.user, &sandbox_name("scratch"))
        .await
        .unwrap();
    assert_eq!(
        body["config"]["kubernetes"]["pod"]["metadata"]["name"],
        scratch.sandbox.id.0
    );

    for n in 2..MAX_SANDBOXES {
        w.world.sandbox(&w.adam.user, &format!("box-{n}")).await;
    }
    assert_eq!(
        config(&app, "extra.adam", "extra.adam").await,
        (StatusCode::FORBIDDEN, json!({}))
    );
    let names: Vec<String> = w
        .world
        .sandboxes
        .list(&w.adam.user)
        .await
        .unwrap()
        .into_iter()
        .map(|view| view.sandbox.name.to_string())
        .collect();
    assert_eq!(names, ["main", "scratch", "box-2"]);
}

/// Holds every create until the test adds a permit, as an image pull would.
struct SlowCreates {
    engine: FakeEngine,
    pulled: Semaphore,
}

#[async_trait]
impl SandboxEngine for SlowCreates {
    async fn create(&self, id: &SandboxId, spec: &SandboxSpec) -> Result<()> {
        drop(self.pulled.acquire().await.unwrap());
        self.engine.create(id, spec).await
    }
    async fn start(&self, id: &SandboxId, boot: Boot, deadline: Duration) -> Result<Started> {
        self.engine.start(id, boot, deadline).await
    }
    async fn stop(&self, id: &SandboxId) -> Result<()> {
        self.engine.stop(id).await
    }
    async fn delete(&self, id: &SandboxId) -> Result<()> {
        self.engine.delete(id).await
    }
    async fn status(&self, id: &SandboxId) -> Result<SandboxStatus> {
        self.engine.status(id).await
    }
    async fn inspect(&self, id: &SandboxId) -> Result<Inspection> {
        self.engine.inspect(id).await
    }
}

#[tokio::test]
async fn a_sandbox_a_login_creates_is_finished_after_containerssh_stops_waiting() {
    let engine = Arc::new(SlowCreates {
        engine: FakeEngine::default(),
        pulled: Semaphore::new(0),
    });
    let w = World::new();
    w.member("adam").await;
    let users = w.users.clone();
    let sandboxes = Sandboxes::new(
        Arc::new(MemorySandboxRepo::default()),
        users.clone(),
        engine.clone(),
        Credentials::new(Arc::new(MemorySecretStore::default()), users.clone()),
        catalog(),
        Limits {
            max_sandboxes_per_user: MAX_SANDBOXES,
            wake_deadline: WAKE_DEADLINE,
        },
        links(),
    );
    let access = SshAccess::new(users, Arc::new(MemoryKeyRepo::default()), sandboxes, vec![]);
    let app = router(Arc::new(access), ContainerSshConfig::default());

    let given_up = timeout(
        Duration::from_millis(200),
        config(&app, "new.adam", "new.adam"),
    );
    assert!(given_up.await.is_err());
    engine.pulled.add_permits(1);
    let finished = async {
        while !engine
            .engine
            .calls()
            .iter()
            .any(|call| matches!(call, EngineCall::Start { .. }))
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    timeout(Duration::from_secs(5), finished).await.unwrap();
    assert_eq!(config(&app, "new.adam", "new.adam").await.0, StatusCode::OK);
}

struct BrokenUsers;

#[async_trait]
impl UserRepo for BrokenUsers {
    async fn by_subject(&self, _subject: &str) -> Result<Option<User>> {
        Err(Error::internal("store", "down"))
    }
    async fn by_handle(&self, _handle: &Handle) -> Result<Option<User>> {
        Err(Error::internal("store", "down"))
    }
    async fn get(&self, _id: UserId) -> Result<Option<User>> {
        Err(Error::internal("store", "down"))
    }
    async fn insert(&self, _user: &User) -> Result<()> {
        Err(Error::internal("store", "down"))
    }
    async fn update(&self, _user: &User) -> Result<()> {
        Err(Error::internal("store", "down"))
    }
    async fn list(&self) -> Result<Vec<User>> {
        Err(Error::internal("store", "down"))
    }
    async fn delete(&self, _id: UserId) -> Result<bool> {
        Err(Error::internal("store", "down"))
    }
    async fn credentials_changed_at(&self, _id: UserId) -> Result<Option<DateTime<Utc>>> {
        Err(Error::internal("store", "down"))
    }
    async fn record_credentials_change(&self, _id: UserId, _at: DateTime<Utc>) -> Result<()> {
        Err(Error::internal("store", "down"))
    }
    async fn sandbox_defaults(&self, _id: UserId) -> Result<Option<SandboxDefaults>> {
        Err(Error::internal("store", "down"))
    }
    async fn set_sandbox_defaults(&self, _id: UserId, _defaults: &SandboxDefaults) -> Result<()> {
        Err(Error::internal("store", "down"))
    }
}

#[tokio::test]
async fn store_errors_never_become_success() {
    let w = Fixture::new().await;
    let access = Arc::new(SshAccess::new(
        Arc::new(BrokenUsers),
        Arc::new(MemoryKeyRepo::default()),
        w.world.sandboxes.clone(),
        Vec::new(),
    ));
    let app = router(access, ContainerSshConfig::default());
    let adam = w.adam.public_key();
    for _ in 0..12 {
        assert_eq!(
            pubkey(&app, "main.adam", &adam, "1.2.3.4:50022", "c1").await,
            (StatusCode::INTERNAL_SERVER_ERROR, json!({}))
        );
    }
    assert_eq!(
        config(&app, "main.adam", "main.adam").await,
        (StatusCode::INTERNAL_SERVER_ERROR, json!({}))
    );
}

#[tokio::test]
async fn every_response_is_json() {
    let w = Fixture::new().await;
    let app = w.router();
    let missing_address = json!({"connectionId": "c", "username": "adam", "publicKey": "k"});
    let bad_address = json!({"remoteAddress": "nowhere", "connectionId": "c", "username": "adam", "publicKey": "k"});
    for (method, path, body, status) in [
        (
            Method::POST,
            "/pubkey",
            "{".to_owned(),
            StatusCode::BAD_REQUEST,
        ),
        (
            Method::POST,
            "/pubkey",
            missing_address.to_string(),
            StatusCode::BAD_REQUEST,
        ),
        (
            Method::POST,
            "/pubkey",
            bad_address.to_string(),
            StatusCode::BAD_REQUEST,
        ),
        (
            Method::POST,
            "/config",
            json!({"username": "adam"}).to_string(),
            StatusCode::BAD_REQUEST,
        ),
        (
            Method::GET,
            "/pubkey",
            String::new(),
            StatusCode::METHOD_NOT_ALLOWED,
        ),
        (
            Method::POST,
            "/password",
            String::new(),
            StatusCode::NOT_FOUND,
        ),
    ] {
        assert_eq!(
            send(&app, method.clone(), path, body).await,
            (status, json!({})),
            "{method} {path}"
        );
    }
}
