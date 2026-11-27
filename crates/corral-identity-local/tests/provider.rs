use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use axum::http::StatusCode;
use corral_api::router;
use corral_api::testing::{Browser, Reply, api, trusting_provider};
use corral_core::domain::{Handle, Role};
use corral_core::fakes::{Logs, PROVIDER, World};
use corral_identity_local::LocalProvider;
use corral_sql::Pool;
use corral_sql::testing::Databases;
use secrecy::SecretString;
use serde_json::{Value, json};

const PASSWORD: &str = "correct horse battery";
const TWELVE_CHARS: &str = "twelve chars";

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

async fn change_password(browser: &mut Browser, current: &str, password: &str) -> Reply {
    let change = json!({ "current": current, "password": password });
    browser
        .call("PUT", "/auth/local/password", Some(change))
        .await
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
        json!({
            "handle": "adam",
            "email": null,
            "picture": null,
            "role": "member",
            "provider": "local",
        })
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
            "accounts": "/auth/local/accounts",
            "opkssh": null,
        }])
    );
}

async fn wrong_password_and_unknown_user_are_both_forbidden(pool: Pool) {
    let corral = Corral::open(pool, &[("adam", Role::Member)]).await;
    let logs = Logs::capture();
    for (username, attempt) in [
        ("adam", "wrong horse battery"),
        ("nobody", PASSWORD),
        ("Not A Handle", PASSWORD),
        ("Adam", PASSWORD),
    ] {
        let mut browser = corral.browser();
        let reply = log_in(&mut browser, username, attempt).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "{username}");
        assert_eq!(
            reply.json(),
            json!({ "error": "wrong username or password" })
        );
        assert_eq!(
            browser.get("/api/me").await.status,
            StatusCode::UNAUTHORIZED
        );
    }
    let refused = logs.of("a sign-in failed");
    let usernames: Vec<_> = refused
        .iter()
        .map(|line| {
            assert!(line.starts_with(" WARN"), "{line}");
            assert!(
                line.contains("provider=local")
                    && line.ends_with("reason=\"wrong username or password\""),
                "{line}"
            );
            line.split("username=")
                .nth(1)
                .unwrap()
                .split(" reason=")
                .next()
                .unwrap()
        })
        .collect();
    assert_eq!(
        usernames,
        ["\"adam\"", "\"nobody\"", "\"Not A Handle\"", "\"Adam\""]
    );
    assert!(
        !logs.lines().join("\n").contains("horse"),
        "{:#?}",
        logs.lines()
    );
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

async fn the_admin_asking_sees_which_account_is_theirs(pool: Pool) {
    let corral = Corral::open(
        pool,
        &[
            ("root", Role::Admin),
            ("adam", Role::Admin),
            ("never", Role::Member),
        ],
    )
    .await;
    for admin in ["root", "adam"] {
        let mut browser = corral.signed_in(admin).await;
        let accounts = browser.get("/auth/local/accounts").await.json();
        let yours: Vec<&str> = accounts
            .as_array()
            .unwrap()
            .iter()
            .filter(|account| account["yours"] == true)
            .map(|account| account["username"].as_str().unwrap())
            .collect();
        assert_eq!(yours, [admin]);
    }
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

async fn a_disabled_user_is_told_so_with_the_right_password_only(pool: Pool) {
    let corral = Corral::open(pool, &[("root", Role::Admin), ("adam", Role::Member)]).await;
    let mut root = corral.signed_in("root").await;
    let mut adam = corral.signed_in("adam").await;
    let users = root.get("/api/admin/users").await.json();
    let id = users
        .as_array()
        .unwrap()
        .iter()
        .find(|user| user["handle"] == "adam")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let disable = format!("/api/admin/users/{id}/disable");
    assert_eq!(root.post(&disable, json!({})).await.status, StatusCode::OK);

    let disabled = json!({ "error": "this account is disabled" });
    let session = adam.get("/api/me").await;
    assert_eq!(
        (session.status, session.json()),
        (StatusCode::FORBIDDEN, disabled.clone())
    );
    let mut again = corral.browser();
    let reply = log_in(&mut again, "adam", PASSWORD).await;
    assert_eq!(
        (reply.status, reply.json()),
        (StatusCode::FORBIDDEN, disabled)
    );
    let wrong = log_in(&mut again, "adam", "wrong horse battery").await;
    assert_eq!(
        wrong.json(),
        json!({ "error": "wrong username or password" })
    );

    let enable = format!("/api/admin/users/{id}/enable");
    assert_eq!(root.post(&enable, json!({})).await.status, StatusCode::OK);
    corral.signed_in("adam").await;
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

async fn a_user_changes_their_own_password(pool: Pool) {
    let corral = Corral::open(pool, &[("bob", Role::Member)]).await;
    let mut bob = corral.signed_in("bob").await;
    let changed = change_password(&mut bob, PASSWORD, TWELVE_CHARS).await;
    assert_eq!(changed.status, StatusCode::NO_CONTENT, "{}", changed.text());
    let mut again = corral.browser();
    assert_eq!(
        log_in(&mut again, "bob", TWELVE_CHARS).await.status,
        StatusCode::OK
    );
    let mut old = corral.browser();
    assert_eq!(
        log_in(&mut old, "bob", PASSWORD).await.status,
        StatusCode::FORBIDDEN
    );
}

async fn the_current_password_must_match(pool: Pool) {
    let corral = Corral::open(pool, &[("bob", Role::Member)]).await;
    let mut bob = corral.signed_in("bob").await;
    let refused = change_password(&mut bob, "wrong horse battery", TWELVE_CHARS).await;
    assert_eq!(
        (refused.status, refused.json()),
        (StatusCode::FORBIDDEN, json!({ "error": "wrong password" }))
    );
    corral.signed_in("bob").await;

    for _ in 0..4 {
        change_password(&mut bob, "wrong horse battery", TWELVE_CHARS).await;
    }
    let limited = json!({ "error": "too many failed logins, try again later" });
    let change = change_password(&mut bob, PASSWORD, TWELVE_CHARS).await;
    assert_eq!(
        (change.status, change.json()),
        (StatusCode::TOO_MANY_REQUESTS, limited.clone())
    );
    let mut elsewhere = corral.browser();
    let login = log_in(&mut elsewhere, "bob", PASSWORD).await;
    assert_eq!(
        (login.status, login.json()),
        (StatusCode::TOO_MANY_REQUESTS, limited)
    );
}

async fn a_short_new_password_is_refused(pool: Pool) {
    let corral = Corral::open(pool, &[("bob", Role::Member)]).await;
    let mut bob = corral.signed_in("bob").await;
    let refused = change_password(&mut bob, PASSWORD, "elevenchars").await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST);
    let error = refused.json()["error"].as_str().unwrap().to_owned();
    assert!(error.contains("at least 12 characters"), "{error}");
    corral.signed_in("bob").await;
}

async fn a_user_of_another_provider_has_no_password_to_change(pool: Pool) {
    let local = Arc::new(LocalProvider::open(pool).await.unwrap());
    let world = World::with_providers(vec![local.clone()]);
    let app = router(
        api(&world),
        vec![local.routes(), trusting_provider(PROVIDER)],
    );
    let mut eve = Browser::new(&app, "192.0.2.1:5000");
    let login = json!({ "subject": "eve", "username": "eve", "role": "member" });
    assert_eq!(
        eve.post("/auth/test/login", login).await.status,
        StatusCode::OK
    );
    let refused = change_password(&mut eve, PASSWORD, TWELVE_CHARS).await;
    assert_eq!(
        (refused.status, refused.json()),
        (
            StatusCode::BAD_REQUEST,
            json!({ "error": "your account does not sign in with a password" })
        )
    );
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
    the_admin_asking_sees_which_account_is_theirs(databases.pool().await).await;
    short_passwords_are_refused(databases.pool().await).await;
    deleting_an_account_deletes_its_user_and_frees_the_username(databases.pool().await).await;
    an_account_that_never_logged_in_is_only_forgotten(databases.pool().await).await;
    a_disabled_user_is_told_so_with_the_right_password_only(databases.pool().await).await;
    a_user_deleting_themselves_loses_their_account(databases.pool().await).await;
    only_admins_manage_accounts(databases.pool().await).await;
    a_user_changes_their_own_password(databases.pool().await).await;
    the_current_password_must_match(databases.pool().await).await;
    a_short_new_password_is_refused(databases.pool().await).await;
    a_user_of_another_provider_has_no_password_to_change(databases.pool().await).await;
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
