use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use axum::http::StatusCode;
use corral_api::router;
use corral_api::testing::{Browser, Reply, api};
use corral_core::domain::{Handle, Role};
use corral_core::fakes::World;
use corral_identity_local::LocalProvider;
use corral_sql::testing::Databases;
use secrecy::SecretString;
use serde_json::json;

const PASSWORD: &str = "correct horse battery";

/// Adam and Eve, members; `trust_forwarded_for` as given.
async fn app(databases: &Databases, trust_forwarded_for: bool) -> Router {
    let local = Arc::new(LocalProvider::open(databases.pool().await).await.unwrap());
    for name in ["adam", "eve"] {
        local
            .create(
                &Handle::parse(name).unwrap(),
                SecretString::from(PASSWORD),
                Role::Member,
            )
            .await
            .unwrap();
    }
    let world = World::with_providers(vec![local.clone()]);
    let mut api = api(&world);
    api.trust_forwarded_for = trust_forwarded_for;
    router(api, vec![local.routes()])
}

async fn log_in(browser: &mut Browser, username: &str, password: &str) -> Reply {
    let form = json!({ "username": username, "password": password });
    browser.post("/auth/local/login", form).await
}

async fn log_in_via(browser: &mut Browser, username: &str, forwarded_for: &str) -> StatusCode {
    let request = axum::http::Request::post("/auth/local/login")
        .header("content-type", "application/json")
        .header("x-forwarded-for", forwarded_for)
        .body(axum::body::Body::from(
            json!({ "username": username, "password": PASSWORD }).to_string(),
        ))
        .unwrap();
    browser.send(request).await.status
}

#[tokio::test]
async fn the_sixth_failure_for_an_account_is_429_without_a_password_check() {
    let databases = Databases::sqlite();
    let app = app(&databases, false).await;
    let mut checked = Duration::MAX;
    for attempt in 1..=5 {
        let mut attacker = Browser::new(&app, &format!("198.51.100.{attempt}:4000"));
        let started = Instant::now();
        let reply = log_in(&mut attacker, "adam", &format!("guess number {attempt}")).await;
        checked = checked.min(started.elapsed());
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "attempt {attempt}");
    }
    let mut attacker = Browser::new(&app, "198.51.100.66:4000");
    for password in ["guess number 6", PASSWORD] {
        let started = Instant::now();
        let reply = log_in(&mut attacker, "adam", password).await;
        let refused = started.elapsed();
        assert_eq!(reply.status, StatusCode::TOO_MANY_REQUESTS, "{password}");
        assert!(reply.json()["error"].is_string());
        assert!(refused * 4 < checked, "{refused:?} vs {checked:?}");
    }
    assert_eq!(
        attacker.get("/api/me").await.status,
        StatusCode::UNAUTHORIZED
    );
    let mut eve = Browser::new(&app, "192.0.2.2:5000");
    assert_eq!(
        log_in(&mut eve, "eve", PASSWORD).await.status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn the_sixth_failure_from_a_network_is_429_for_every_account() {
    let databases = Databases::sqlite();
    let app = app(&databases, false).await;
    let mut attacker = Browser::new(&app, "198.51.100.66:4000");
    for i in 1..=5 {
        let reply = log_in(&mut attacker, &format!("user-{i}"), PASSWORD).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "{i}");
    }
    assert_eq!(
        log_in(&mut attacker, "adam", PASSWORD).await.status,
        StatusCode::TOO_MANY_REQUESTS
    );
    let mut adam = Browser::new(&app, "192.0.2.1:5000");
    assert_eq!(
        log_in(&mut adam, "adam", PASSWORD).await.status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn successful_logins_are_never_limited() {
    let databases = Databases::sqlite();
    let app = app(&databases, false).await;
    let mut browser = Browser::new(&app, "192.0.2.10:50000");
    for attempt in 1..=10 {
        let reply = log_in(&mut browser, "adam", PASSWORD).await;
        assert_eq!(reply.status, StatusCode::OK, "attempt {attempt}");
    }
}

#[tokio::test]
async fn a_successful_login_does_not_reset_the_failures() {
    let databases = Databases::sqlite();
    let app = app(&databases, false).await;
    let mut attacker = Browser::new(&app, "198.51.100.66:4000");
    for attempt in 1..=4 {
        let reply = log_in(&mut attacker, "adam", &format!("guess {attempt}")).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "attempt {attempt}");
    }
    assert_eq!(
        log_in(&mut attacker, "eve", PASSWORD).await.status,
        StatusCode::OK
    );
    assert_eq!(
        log_in(&mut attacker, "adam", "guess 5").await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        log_in(&mut attacker, "eve", PASSWORD).await.status,
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn behind_a_trusted_proxy_each_client_is_counted() {
    let databases = Databases::sqlite();
    let app = app(&databases, true).await;
    let mut proxy = Browser::new(&app, "10.0.0.2:40000");
    for i in 1..=5 {
        let status = log_in_via(&mut proxy, &format!("user-{i}"), "203.0.113.7").await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{i}");
    }
    for spoofed in ["203.0.113.7", "198.51.100.9, 203.0.113.7"] {
        let status = log_in_via(&mut proxy, "adam", spoofed).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{spoofed}");
    }
    let status = log_in_via(&mut proxy, "adam", "203.0.113.7, 198.51.100.9").await;
    assert_eq!(status, StatusCode::OK);
}
