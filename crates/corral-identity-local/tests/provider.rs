use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use axum::http::StatusCode;
use corral_api::router;
use corral_api::testing::{Browser, Reply, api};
use corral_core::domain::{Handle, Role};
use corral_core::fakes::World;
use corral_identity_local::LocalProvider;
use corral_sql::Pool;
use corral_sql::testing::Databases;
use secrecy::SecretString;
use serde_json::{Value, json};

const PASSWORD: &str = "correct horse battery";

struct Corral {
    app: Router,
    local: Arc<LocalProvider>,
    peers: std::cell::Cell<u8>,
}

impl Corral {
    async fn open(pool: Pool, accounts: &[(&str, Role)]) -> Self {
        let local = Arc::new(LocalProvider::open(pool).await.unwrap());
        for &(username, role) in accounts {
            local
                .create(&handle(username), SecretString::from(PASSWORD), role)
                .await
                .unwrap();
        }
        let world = World::with_providers(vec![local.clone()]);
        Self {
            app: router(api(&world), vec![local.routes()]),
            local,
            peers: std::cell::Cell::new(0),
        }
    }

    /// A browser at an address of its own, so failures from one test step never limit another.
    fn browser(&self) -> Browser {
        self.peers.set(self.peers.get() + 1);
        self.browser_at(&format!("192.0.2.{}:5000", self.peers.get()))
    }

    fn browser_at(&self, peer: &str) -> Browser {
        Browser::new(&self.app, peer)
    }

    async fn signed_in(&self, username: &str) -> Browser {
        let mut browser = self.browser();
        let reply = log_in(&mut browser, username, PASSWORD).await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
        browser
    }
}

fn handle(value: &str) -> Handle {
    Handle::parse(value).unwrap()
}

async fn log_in(browser: &mut Browser, username: &str, password: &str) -> Reply {
    let form = json!({ "username": username, "password": password });
    browser.post("/auth/local/login", form).await
}

fn usernames(accounts: &Value) -> Vec<&str> {
    accounts
        .as_array()
        .unwrap()
        .iter()
        .map(|account| account["username"].as_str().unwrap())
        .collect()
}

async fn the_account_decides_the_role(pool: Pool) {
    let corral = Corral::open(pool, &[("adam", Role::Member), ("root", Role::Admin)]).await;
    let mut adam = corral.browser();
    let reply = log_in(&mut adam, "adam", PASSWORD).await;
    assert_eq!(
        reply.json(),
        json!({ "handle": "adam", "email": null, "picture": null, "role": "member" })
    );
    assert_eq!(adam.get("/api/me").await.json()["role"], "member");
    let mut root = corral.signed_in("root").await;
    assert_eq!(root.get("/api/me").await.json()["role"], "admin");
    assert_eq!(
        root.get("/auth/providers").await.json(),
        json!([{
            "id": "local",
            "label": "Local account",
            "kind": "password",
            "login": "/auth/local/login",
        }])
    );
}

async fn wrong_password_and_unknown_user_are_both_forbidden(pool: Pool) {
    let corral = Corral::open(pool, &[("adam", Role::Member)]).await;
    for (username, attempt) in [
        ("adam", "wrong horse battery"),
        ("nobody", PASSWORD),
        ("Not A Handle", PASSWORD),
        ("Adam", PASSWORD),
    ] {
        let mut browser = corral.browser();
        let reply = log_in(&mut browser, username, attempt).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "{username}");
        assert_eq!(reply.json(), json!({ "error": "forbidden" }));
        assert_eq!(
            browser.get("/api/me").await.status,
            StatusCode::UNAUTHORIZED
        );
    }
}

async fn fastest_refusal(corral: &Corral, username: &str) -> Duration {
    let mut best = Duration::MAX;
    for _ in 0..3 {
        let mut browser = corral.browser();
        let started = Instant::now();
        let reply = log_in(&mut browser, username, "wrong horse battery").await;
        best = best.min(started.elapsed());
        assert_eq!(reply.status, StatusCode::FORBIDDEN);
    }
    best
}

async fn unknown_usernames_cost_a_full_password_check(pool: Pool) {
    let corral = Corral::open(pool, &[("adam", Role::Member)]).await;
    let known = fastest_refusal(&corral, "adam").await;
    for unknown in ["nobody", "Not A Handle"] {
        let elapsed = fastest_refusal(&corral, unknown).await;
        assert!(elapsed * 4 > known, "{unknown}: {elapsed:?} vs {known:?}");
    }
}

async fn admins_create_list_and_reset_accounts(pool: Pool) {
    // Byte order puts '-' before digits, unlike most locale collations.
    let corral = Corral::open(
        pool,
        &[
            ("root", Role::Admin),
            ("adam1", Role::Member),
            ("adam-b", Role::Admin),
        ],
    )
    .await;
    let mut root = corral.signed_in("root").await;
    let created = root
        .post(
            "/auth/local/accounts",
            json!({ "username": "eve", "password": "eve's first password", "role": "member" }),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.text());
    let duplicate = root
        .post(
            "/auth/local/accounts",
            json!({ "username": "eve", "password": "another password", "role": "admin" }),
        )
        .await;
    assert_eq!(duplicate.status, StatusCode::CONFLICT);
    assert_eq!(
        duplicate.json(),
        json!({ "error": "conflict: account eve already exists" })
    );
    assert!(matches!(
        corral
            .local
            .create(&handle("eve"), SecretString::from(PASSWORD), Role::Admin)
            .await,
        Err(corral_core::Error::Conflict(message)) if message == "account eve already exists"
    ));

    let reset = root
        .call(
            "PUT",
            "/auth/local/accounts/eve/password",
            Some(json!({ "password": "eve's second password" })),
        )
        .await;
    assert_eq!(reset.status, StatusCode::NO_CONTENT);
    let mut eve = corral.browser();
    assert_eq!(
        log_in(&mut eve, "eve", "eve's first password").await.status,
        StatusCode::FORBIDDEN
    );
    let mut eve = corral.browser();
    let signed_in = log_in(&mut eve, "eve", "eve's second password").await;
    assert_eq!(signed_in.json()["role"], "member");
    for absent in ["ghost", "Not_A_Handle"] {
        let reply = root
            .call(
                "PUT",
                &format!("/auth/local/accounts/{absent}/password"),
                Some(json!({ "password": "a long enough password" })),
            )
            .await;
        assert_eq!(reply.status, StatusCode::NOT_FOUND, "{absent}");
    }

    let accounts = root.get("/auth/local/accounts").await;
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
            json!(["adam-b", "admin"]),
            json!(["adam1", "member"]),
            json!(["eve", "member"]),
            json!(["root", "admin"]),
        ]
    );
    assert!(accounts.json()[0]["createdAt"].is_string());
}

async fn short_passwords_are_refused(pool: Pool) {
    let corral = Corral::open(pool, &[("root", Role::Admin)]).await;
    let mut root = corral.signed_in("root").await;
    let created = root
        .post(
            "/auth/local/accounts",
            json!({ "username": "eve", "password": "elevenchars", "role": "member" }),
        )
        .await;
    assert_eq!(created.status, StatusCode::BAD_REQUEST);
    assert!(!created.text().contains("elevenchars"));
    let reset = root
        .call(
            "PUT",
            "/auth/local/accounts/root/password",
            Some(json!({ "password": "elevenchars" })),
        )
        .await;
    assert_eq!(reset.status, StatusCode::BAD_REQUEST);
    let mut eve = corral.browser();
    assert_eq!(
        log_in(&mut eve, "eve", "elevenchars").await.status,
        StatusCode::FORBIDDEN
    );
    corral.signed_in("root").await;
}

async fn deleting_an_account_deletes_its_user_and_frees_the_username(pool: Pool) {
    let corral = Corral::open(pool, &[("root", Role::Admin), ("adam", Role::Member)]).await;
    let mut root = corral.signed_in("root").await;
    let mut adam = corral.signed_in("adam").await;
    let created = adam.post("/api/sandboxes", json!({ "name": "main" })).await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.text());

    let deleted = root.call("DELETE", "/auth/local/accounts/adam", None).await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT, "{}", deleted.text());
    assert_eq!(adam.get("/api/me").await.status, StatusCode::FORBIDDEN);
    assert_eq!(root.get("/api/admin/sandboxes").await.json(), json!([]));
    let users = root.get("/api/admin/users").await.json();
    assert!(!users.to_string().contains("adam"), "{users}");
    assert_eq!(
        usernames(&root.get("/auth/local/accounts").await.json()),
        ["root"]
    );
    let mut again = corral.browser();
    assert_eq!(
        log_in(&mut again, "adam", PASSWORD).await.status,
        StatusCode::FORBIDDEN
    );
    for absent in ["adam", "Not_A_Handle"] {
        let reply = root
            .call("DELETE", &format!("/auth/local/accounts/{absent}"), None)
            .await;
        assert_eq!(reply.status, StatusCode::NOT_FOUND, "{absent}");
    }

    let reused = root
        .post(
            "/auth/local/accounts",
            json!({ "username": "adam", "password": "a newcomer's password", "role": "member" }),
        )
        .await;
    assert_eq!(reused.status, StatusCode::CREATED);
    let mut newcomer = corral.browser();
    let reply = log_in(&mut newcomer, "adam", "a newcomer's password").await;
    assert_eq!(reply.json()["handle"], "adam");
    assert_eq!(newcomer.get("/api/sandboxes").await.json(), json!([]));
}

async fn an_account_that_never_logged_in_is_only_forgotten(pool: Pool) {
    let corral = Corral::open(pool, &[("root", Role::Admin), ("never", Role::Member)]).await;
    let mut root = corral.signed_in("root").await;
    let before = root.get("/api/admin/users").await.json();
    let deleted = root
        .call("DELETE", "/auth/local/accounts/never", None)
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    assert_eq!(root.get("/api/admin/users").await.json(), before);
    assert_eq!(
        usernames(&root.get("/auth/local/accounts").await.json()),
        ["root"]
    );
}

async fn a_user_deleting_themselves_loses_their_account(pool: Pool) {
    let corral = Corral::open(pool, &[("root", Role::Admin), ("adam", Role::Member)]).await;
    let mut adam = corral.signed_in("adam").await;
    let deleted = adam.call("DELETE", "/api/me", None).await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    let mut root = corral.signed_in("root").await;
    assert_eq!(
        usernames(&root.get("/auth/local/accounts").await.json()),
        ["root"]
    );
}

async fn only_admins_manage_accounts(pool: Pool) {
    let corral = Corral::open(pool, &[("adam", Role::Member)]).await;
    let routes = [
        ("GET", "/auth/local/accounts", None),
        (
            "POST",
            "/auth/local/accounts",
            Some(
                json!({ "username": "mallory", "password": "a long enough password", "role": "admin" }),
            ),
        ),
        (
            "PUT",
            "/auth/local/accounts/adam/password",
            Some(json!({ "password": "a long enough password" })),
        ),
        ("DELETE", "/auth/local/accounts/adam", None),
    ];
    let mut adam = corral.signed_in("adam").await;
    let mut anonymous = corral.browser();
    for (method, uri, body) in routes {
        let reply = anonymous.call(method, uri, body.clone()).await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{method} {uri}");
        let reply = adam.call(method, uri, body).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "{method} {uri}");
    }
    let mut mallory = corral.browser();
    assert_eq!(
        log_in(&mut mallory, "mallory", "a long enough password")
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    corral.signed_in("adam").await;
}

async fn reopening_keeps_the_accounts(pool: Pool) {
    Corral::open(pool.clone(), &[("adam", Role::Member)]).await;
    let reopened = Corral::open(pool, &[]).await;
    reopened.signed_in("adam").await;
}

async fn every_scenario(databases: &Databases) {
    the_account_decides_the_role(databases.pool().await).await;
    wrong_password_and_unknown_user_are_both_forbidden(databases.pool().await).await;
    unknown_usernames_cost_a_full_password_check(databases.pool().await).await;
    admins_create_list_and_reset_accounts(databases.pool().await).await;
    short_passwords_are_refused(databases.pool().await).await;
    deleting_an_account_deletes_its_user_and_frees_the_username(databases.pool().await).await;
    an_account_that_never_logged_in_is_only_forgotten(databases.pool().await).await;
    a_user_deleting_themselves_loses_their_account(databases.pool().await).await;
    only_admins_manage_accounts(databases.pool().await).await;
    reopening_keeps_the_accounts(databases.pool().await).await;
}

#[tokio::test]
async fn on_sqlite() {
    every_scenario(&Databases::sqlite()).await;
}

#[tokio::test]
async fn on_postgres() {
    every_scenario(&Databases::postgres().await).await;
}
