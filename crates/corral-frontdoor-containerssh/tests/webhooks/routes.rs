use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode, header};
use corral_core::app::{Credentials, Limits, RegisteredKeyVerifier, Sandboxes, SshAccess};
use corral_core::domain::{Handle, Role, User, UserId};
use corral_core::fakes::{MemoryKeyRepo, MemorySandboxRepo, MemorySecretStore};
use corral_core::ports::{Readiness, SshTarget, UserRepo};
use corral_core::{Error, Result};
use corral_frontdoor_containerssh::{FrontDoorConfig, LockoutPolicy, router};
use secrecy::SecretString;
use serde_json::{Value, json};
use tower::ServiceExt;

use crate::common::{
    Member, NAMESPACE, World, catalog, generate_key, identity, private_pem, public_line,
};

fn app(access: Arc<SshAccess>, proxy_private_key: &str) -> Router {
    router(
        access,
        FrontDoorConfig {
            proxy_private_key: SecretString::from(proxy_private_key),
            lockout: LockoutPolicy::default(),
        },
    )
}

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

async fn disable(world: &World, member: &Member) {
    let root = world
        .accounts
        .login(identity("root", Role::Admin))
        .await
        .unwrap();
    world
        .accounts
        .set_enabled(&root, member.user.id, false)
        .await
        .unwrap();
}

#[tokio::test]
async fn pubkey_accepts_the_owners_key() {
    let w = World::new().await;
    let app = app(w.access.clone(), "PROXY");
    for username in ["adam", "adam+main"] {
        assert!(accepted(&app, username, &w.adam.public_key(), "1.2.3.4:50022").await);
    }
    assert!(accepted(&app, "eve+main", &w.eve.public_key(), "1.2.3.4:50022").await);
}

#[tokio::test]
async fn pubkey_denies_with_200_and_false() {
    let w = World::new().await;
    let app = app(w.access.clone(), "PROXY");
    let adam = w.adam.public_key();
    let eve = w.eve.public_key();
    let stranger = public_line(&generate_key());
    for (username, key) in [
        ("adam+main", stranger.as_str()),
        ("adam+main", eve.as_str()),
        ("adam", eve.as_str()),
        ("eve+main", adam.as_str()),
        ("nobody", adam.as_str()),
        ("adam+missing", adam.as_str()),
        ("ADAM", adam.as_str()),
        ("adam+main", "not a key"),
    ] {
        assert!(
            !accepted(&app, username, key, "1.2.3.4:50022").await,
            "{username}"
        );
    }
    disable(&w, &w.adam).await;
    assert!(!accepted(&app, "adam+main", &adam, "1.2.3.4:50022").await);
}

#[tokio::test(start_paused = true)]
async fn pubkey_locks_out_an_address_after_repeated_failures() {
    let w = World::new().await;
    let app = app(w.access.clone(), "PROXY");
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

    tokio::time::advance(LockoutPolicy::default().window).await;
    assert!(accepted(&app, "adam", &adam, "1.2.3.4:40003").await);
}

#[tokio::test]
async fn pubkey_counts_a_connection_offering_many_keys_once() {
    let w = World::new().await;
    let app = app(w.access.clone(), "PROXY");
    let unregistered: Vec<String> = (0..3).map(|_| public_line(&generate_key())).collect();
    for connection in 0..9 {
        let address = format!("1.2.3.4:{}", 50000 + connection);
        for key in &unregistered {
            pubkey(&app, "adam", key, &address, &format!("c{connection}")).await;
        }
    }
    assert!(accepted(&app, "adam", &w.adam.public_key(), "1.2.3.4:40000").await);
}

#[tokio::test(start_paused = true)]
async fn pubkey_lockout_policy_is_configurable() {
    let w = World::new().await;
    let app = router(
        w.access.clone(),
        FrontDoorConfig {
            proxy_private_key: SecretString::from("PROXY"),
            lockout: LockoutPolicy {
                max_failed_connections: NonZeroUsize::new(2).unwrap(),
                window: Duration::from_secs(60),
            },
        },
    );
    let stranger = public_line(&generate_key());
    for connection in ["a", "b"] {
        pubkey(&app, "adam", &stranger, "1.2.3.4:50000", connection).await;
    }
    assert!(!accepted(&app, "adam", &w.adam.public_key(), "1.2.3.4:40000").await);
    tokio::time::advance(Duration::from_secs(60)).await;
    assert!(accepted(&app, "adam", &w.adam.public_key(), "1.2.3.4:40000").await);
}

#[tokio::test]
async fn config_answers_each_target_kind() {
    let w = World::new().await;
    let proxy_key = private_pem(&generate_key());
    let app = app(w.access.clone(), &proxy_key);
    w.engine.set_target(
        &w.eve.main.id,
        SshTarget::SshServer {
            host: "corral-sbx-eve".to_owned(),
            port: 22,
            username: "sandbox".to_owned(),
            host_key_fingerprint: "SHA256:abc".to_owned(),
        },
    );

    let (status, body) = config(&app, "adam+main", "adam+main").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        json!({"config": {"backend": "kubernetes", "kubernetes": {"pod": {
            "metadata": {"name": w.adam.main.id.0, "namespace": NAMESPACE},
            "mode": "persistent",
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
            "privateKey": proxy_key,
            "allowedHostKeyFingerprints": ["SHA256:abc"],
        }}})
    );
}

#[tokio::test]
async fn config_routes_the_authenticated_username() {
    let w = World::new().await;
    let app = app(w.access.clone(), "PROXY");
    let (status, body) = config(&app, "eve+main", "adam+main").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["config"]["kubernetes"]["pod"]["metadata"]["name"],
        w.adam.main.id.0
    );
}

#[tokio::test]
async fn config_returns_503_while_waking() {
    let w = World::new().await;
    let app = app(w.access.clone(), "PROXY");
    w.engine.answer_wakes_with(&[Readiness::StillWaking]);
    assert_eq!(
        config(&app, "adam+main", "adam+main").await,
        (StatusCode::SERVICE_UNAVAILABLE, json!({}))
    );
    assert_eq!(w.engine.woken(), vec![w.adam.main.id.clone()]);
    assert_eq!(
        config(&app, "adam+main", "adam+main").await.0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn config_refuses_unknown_disabled_or_missing_with_403() {
    let w = World::new().await;
    let app = app(w.access.clone(), "PROXY");
    disable(&w, &w.eve).await;
    for username in ["nobody", "adam+missing", "eve", "eve+main", "ADAM", ""] {
        assert_eq!(
            config(&app, username, username).await,
            (StatusCode::FORBIDDEN, json!({})),
            "{username}"
        );
    }
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
}

#[tokio::test]
async fn store_errors_never_become_success() {
    let w = World::new().await;
    let sandboxes = Sandboxes::new(
        Arc::new(MemorySandboxRepo::default()),
        w.engine.clone(),
        Credentials::new(Arc::new(MemorySecretStore::default())),
        catalog(),
        Limits {
            max_sandboxes_per_user: 1,
        },
    );
    let access = Arc::new(SshAccess::new(
        Arc::new(BrokenUsers),
        sandboxes,
        vec![Arc::new(RegisteredKeyVerifier::new(Arc::new(
            MemoryKeyRepo::default(),
        )))],
        Duration::from_secs(1),
    ));
    let app = app(access, "PROXY");
    let adam = w.adam.public_key();
    for _ in 0..12 {
        assert_eq!(
            pubkey(&app, "adam+main", &adam, "1.2.3.4:50022", "c1").await,
            (StatusCode::INTERNAL_SERVER_ERROR, json!({}))
        );
    }
    assert_eq!(
        config(&app, "adam+main", "adam+main").await,
        (StatusCode::INTERNAL_SERVER_ERROR, json!({}))
    );
}

#[tokio::test]
async fn every_response_is_json() {
    let w = World::new().await;
    let app = app(w.access.clone(), "PROXY");
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
