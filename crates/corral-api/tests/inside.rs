use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use corral_api::router;
use corral_api::testing::{Browser, Reply, api, trusting_provider};
use corral_core::app::SANDBOX_FILE;
use corral_core::domain::{Handle, HomePath, Role, Sandbox, SandboxStatus};
use corral_core::fakes::{EngineCall, PROVIDER, World, sandbox_name};
use corral_core::ports::{SandboxEngine, Usage, UserRepo};
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
        self.inside_with(method, path, token, None).await
    }

    async fn inside_with(
        &self,
        method: &str,
        path: &str,
        token: Option<&str>,
        body: Option<Value>,
    ) -> Reply {
        let mut request = Request::builder().method(method).uri(path);
        if method == "POST" {
            request = request.header(header::CONTENT_TYPE, "application/json");
        }
        if let Some(token) = token {
            request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        let body = body.map_or_else(Body::empty, |body| Body::from(body.to_string()));
        let mut browser = Browser::new(&self.app, "198.51.100.7:40000");
        browser.send(request.body(body).unwrap()).await
    }

    async fn delete_from_inside(&self, token: &str, confirm: &str) -> Reply {
        let body = json!({ "confirm": confirm });
        self.inside_with("POST", "/api/sandbox/delete", Some(token), Some(body))
            .await
    }

    /// Waits for the sandbox to be in `status`, which work run on its own reaches.
    async fn reaches(&self, handle: &str, name: &str, status: SandboxStatus) {
        for _ in 0..100 {
            if self.status(handle, name).await == status {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(self.status(handle, name).await, status);
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
            ("POST", "/api/sandbox/delete"),
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

#[tokio::test]
async fn delete_from_inside_is_refused_until_the_owner_turns_it_on() {
    let corral = Corral::new();
    let mut adam = corral.member("adam").await;
    adam.post("/api/sandboxes", json!({ "name": "main" })).await;
    let token = corral.token("adam", "main").await;
    let setting = async |adam: &mut Browser| adam.get("/api/me/delete-from-inside").await.json();
    assert_eq!(setting(&mut adam).await, json!({ "enabled": false }));

    let off = corral.delete_from_inside(&token, "main").await;
    assert_eq!(off.status, StatusCode::FORBIDDEN, "{}", off.text());
    assert_eq!(
        off.json(),
        json!({ "error": "deleting a sandbox from inside it is off; turn it on in your profile \
                          in the web UI" })
    );
    let turned_on = adam
        .call(
            "PUT",
            "/api/me/delete-from-inside",
            Some(json!({ "enabled": true })),
        )
        .await;
    assert_eq!(turned_on.status, StatusCode::NO_CONTENT);
    assert_eq!(setting(&mut adam).await, json!({ "enabled": true }));
    let unconfirmed = corral.delete_from_inside(&token, "other").await;
    assert_eq!(unconfirmed.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        unconfirmed.json(),
        json!({ "error": "invalid: to confirm, type this sandbox's name, main" })
    );
    let shapeless = corral
        .inside_with("POST", "/api/sandbox/delete", Some(&token), Some(json!({})))
        .await;
    assert_eq!(shapeless.status, StatusCode::BAD_REQUEST);
    assert_eq!(corral.status("adam", "main").await, SandboxStatus::Running);
    assert_eq!(
        adam.get("/api/sandboxes/main").await.json()["deleteAt"],
        Value::Null
    );
}

#[tokio::test]
async fn a_deletion_from_inside_stops_the_sandbox_until_cancelled() {
    let corral = Corral::new();
    let mut adam = corral.member("adam").await;
    adam.post("/api/sandboxes", json!({ "name": "main" })).await;
    adam.call(
        "PUT",
        "/api/me/delete-from-inside",
        Some(json!({ "enabled": true })),
    )
    .await;
    let token = corral.token("adam", "main").await;

    let scheduled = corral.delete_from_inside(&token, "main").await;
    assert_eq!(
        scheduled.status,
        StatusCode::ACCEPTED,
        "{}",
        scheduled.text()
    );
    let delete_at = scheduled.json()["deleteAt"].as_str().unwrap().to_owned();
    let in_ten_minutes = Utc::now() + TimeDelta::minutes(10);
    let parsed: DateTime<Utc> = delete_at.parse().unwrap();
    assert!(
        (in_ten_minutes - parsed).abs() < TimeDelta::seconds(5),
        "{delete_at}"
    );
    corral
        .reaches("adam", "main", SandboxStatus::Sleeping)
        .await;
    let shown = adam.get("/api/sandboxes/main").await.json();
    assert_eq!(
        (&shown["state"], &shown["deleteAt"]),
        (&json!("sleeping"), &json!(delete_at))
    );
    assert_eq!(
        corral
            .inside("GET", "/api/sandbox", Some(&token))
            .await
            .status,
        StatusCode::UNAUTHORIZED,
        "the stop revoked the token"
    );
    for refused in ["wake", "restart"] {
        let reply = adam
            .call("POST", &format!("/api/sandboxes/main/{refused}"), None)
            .await;
        assert_eq!(reply.status, StatusCode::CONFLICT, "{refused}");
        assert!(
            reply.json()["error"]
                .as_str()
                .unwrap()
                .contains("cancel the deletion"),
            "{}",
            reply.text()
        );
    }

    let cancelled = adam
        .call("DELETE", "/api/sandboxes/main/deletion", None)
        .await;
    assert_eq!(cancelled.status, StatusCode::NO_CONTENT);
    let shown = adam.get("/api/sandboxes/main").await.json();
    assert_eq!(
        (&shown["state"], &shown["deleteAt"]),
        (&json!("sleeping"), &Value::Null)
    );
    let woken = adam.call("POST", "/api/sandboxes/main/wake", None).await;
    assert_eq!(woken.json(), json!({ "ready": true }));
    let missing = adam
        .call("DELETE", "/api/sandboxes/nope/deletion", None)
        .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_sandbox_reports_what_it_used_and_hears_when_to_report_again() {
    let corral = Corral::new();
    let mut adam = corral.member("adam").await;
    adam.post("/api/sandboxes", json!({ "name": "main" })).await;
    let token = corral.token("adam", "main").await;
    let id = corral.sandbox("adam", "main").await.id;

    let report = json!({ "intervalMs": 60_000, "cpuUs": 1_500_000, "networkBytes": 4096 });
    let reply = corral
        .inside_with("POST", "/api/sandbox/activity", Some(&token), Some(report))
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    assert_eq!(reply.json(), json!({ "afterSecs": 60 }));
    let usage = Usage {
        interval: Duration::from_mins(1),
        cpu: Duration::from_millis(1500),
        network_bytes: 4096,
    };
    assert_eq!(corral.world.signals.reports(), [(id, usage)]);

    let refused = corral
        .inside_with(
            "POST",
            "/api/sandbox/activity",
            Some("corral_sbx_nope"),
            Some(json!({ "intervalMs": 0, "cpuUs": 0, "networkBytes": 0 })),
        )
        .await;
    assert_eq!(refused.status, StatusCode::UNAUTHORIZED);
    assert_eq!(corral.world.signals.reports().len(), 1);
}

#[tokio::test]
async fn a_sandbox_keeps_itself_awake_and_lets_itself_sleep_again() {
    let corral = Corral::new();
    let mut adam = corral.member("adam").await;
    adam.post("/api/sandboxes", json!({ "name": "main" })).await;
    let token = corral.token("adam", "main").await;

    let before = Utc::now();
    let kept = corral
        .inside_with(
            "POST",
            "/api/sandbox/keep-awake",
            Some(&token),
            Some(json!({ "forSecs": 4 * 3600 })),
        )
        .await;
    assert_eq!(kept.status, StatusCode::OK, "{}", kept.text());
    let until: DateTime<Utc> =
        serde_json::from_value(kept.json()["idle"]["keepAwakeUntil"].clone()).unwrap();
    assert!(until >= before + TimeDelta::hours(4) && until <= Utc::now() + TimeDelta::hours(4));
    assert_eq!(
        adam.get("/api/sandboxes/main").await.json()["idle"]["keepAwakeUntil"],
        kept.json()["idle"]["keepAwakeUntil"]
    );

    let too_long = corral
        .inside_with(
            "POST",
            "/api/sandbox/keep-awake",
            Some(&token),
            Some(json!({ "forSecs": 25 * 3600 })),
        )
        .await;
    assert_eq!(
        too_long.status,
        StatusCode::BAD_REQUEST,
        "{}",
        too_long.text()
    );

    let off = corral
        .inside_with(
            "POST",
            "/api/sandbox/keep-awake",
            Some(&token),
            Some(json!({ "forSecs": null })),
        )
        .await;
    assert_eq!(off.json()["idle"]["keepAwakeUntil"], Value::Null);
}

#[tokio::test]
async fn an_owner_sets_the_idle_timeout_and_keeps_a_sandbox_awake() {
    let corral = Corral::new();
    let mut adam = corral.member("adam").await;
    let mut eve = corral.member("eve").await;
    adam.post("/api/sandboxes", json!({ "name": "main" })).await;
    let idle = adam.get("/api/sandboxes/main").await.json()["idle"].clone();
    assert_eq!(
        idle,
        json!({
            "timeoutSecs": 1800,
            "chosen": false,
            "defaultTimeoutSecs": 1800,
            "minTimeoutSecs": 60,
            "maxTimeoutSecs": 4 * 3600,
            "keepAwakeUntil": null,
            "maxKeepAwakeSecs": 24 * 3600,
        })
    );

    let set = adam
        .call(
            "PUT",
            "/api/sandboxes/main/idle-timeout",
            Some(json!({ "timeoutSecs": 7200 })),
        )
        .await;
    assert_eq!(set.status, StatusCode::OK, "{}", set.text());
    assert_eq!(set.json()["idle"]["timeoutSecs"], 7200);
    assert_eq!(set.json()["idle"]["chosen"], true);
    let past_the_maximum = adam
        .call(
            "PUT",
            "/api/sandboxes/main/idle-timeout",
            Some(json!({ "timeoutSecs": 5 * 3600 })),
        )
        .await;
    assert_eq!(past_the_maximum.status, StatusCode::BAD_REQUEST);
    let theirs = eve
        .call(
            "PUT",
            "/api/sandboxes/main/idle-timeout",
            Some(json!({ "timeoutSecs": 7200 })),
        )
        .await;
    assert_eq!(theirs.status, StatusCode::NOT_FOUND);
    let default = adam
        .call(
            "PUT",
            "/api/sandboxes/main/idle-timeout",
            Some(json!({ "timeoutSecs": null })),
        )
        .await;
    assert_eq!(default.json()["idle"]["timeoutSecs"], 1800);

    let until = "2099-01-01T00:00:00Z";
    let too_far = adam
        .call(
            "PUT",
            "/api/sandboxes/main/keep-awake",
            Some(json!({ "until": until })),
        )
        .await;
    assert_eq!(too_far.status, StatusCode::BAD_REQUEST);
    let until = (Utc::now() + TimeDelta::hours(1)).to_rfc3339();
    let kept = adam
        .call(
            "PUT",
            "/api/sandboxes/main/keep-awake",
            Some(json!({ "until": until })),
        )
        .await;
    assert_eq!(kept.status, StatusCode::OK, "{}", kept.text());
    assert!(kept.json()["idle"]["keepAwakeUntil"].is_string());
}
