use axum::Router;
use axum::http::StatusCode;
use corral_api::router;
use corral_api::testing::{Browser, Reply, api, trusting_provider};
use corral_core::domain::Role;
use corral_core::fakes::{PROVIDER, World};
use serde_json::{Value, json};

const SHARED_SETUP: &str = "/api/admin/settings/shared-setup";

struct Corral {
    app: Router,
}

impl Corral {
    fn new() -> Self {
        Self {
            app: router(api(&World::new()), vec![trusting_provider(PROVIDER)]),
        }
    }

    async fn signed_in(&self, handle: &str, role: Role) -> Browser {
        let mut browser = Browser::new(&self.app, "192.0.2.10:50000");
        let login = json!({ "subject": handle, "username": handle, "role": role });
        let reply = browser.post("/auth/test/login", login).await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
        browser
    }
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

fn shared_variable(name: &str, value: &str, shared: &str, secret: bool) -> Value {
    let mut entry = variable(name, value, secret);
    entry["shared"] = json!(shared);
    entry
}

fn created(reply: &Reply) -> Value {
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.text());
    reply.json()
}

fn id(entry: &Value) -> &str {
    entry["id"].as_str().unwrap()
}

fn refused(reply: &Reply, status: StatusCode, error: &str) {
    assert_eq!(reply.status, status, "{}", reply.text());
    assert_eq!(reply.json(), json!({ "error": error }));
}

fn fields<'a>(entry: &'a Value, names: &[&str]) -> Vec<&'a Value> {
    names.iter().map(|name| &entry[*name]).collect()
}

#[tokio::test]
async fn the_admin_defines_a_required_entry_and_members_see_it_locked() {
    let corral = Corral::new();
    let mut root = corral.signed_in("root", Role::Admin).await;
    let mut bob = corral.signed_in("bob", Role::Member).await;
    bob.post("/api/sandboxes", json!({ "name": "main" })).await;
    created(
        &bob.post("/api/setup", variable("EDITOR", "vim", false))
            .await,
    );

    let gateway = created(
        &root
            .post(
                SHARED_SETUP,
                shared_variable("GATEWAY_URL", "https://llm", "required", false),
            )
            .await,
    );
    assert_eq!(
        fields(&gateway, &["target", "shared", "attached", "shadowedBy"]),
        [
            &json!("GATEWAY_URL"),
            &json!("required"),
            &json!(0),
            &json!([])
        ]
    );
    assert_eq!(gateway["value"]["text"], "https://llm");

    let listed = bob.get("/api/setup").await.json();
    let marks: Vec<Vec<&Value>> = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| fields(entry, &["target", "shared", "shadows", "shadowed"]))
        .collect();
    assert_eq!(
        marks,
        [
            [
                &json!("GATEWAY_URL"),
                &json!("required"),
                &json!(false),
                &json!(false)
            ],
            [&json!("EDITOR"), &json!(null), &json!(false), &json!(false)],
        ]
    );
    assert_eq!(listed[0]["sandboxes"][0]["sandbox"], "main");
    bob.call(
        "PUT",
        &format!("/api/sandboxes/main/setup/{}", id(&listed[1])),
        None,
    )
    .await;
    let held = bob.get("/api/sandboxes/main/setup").await.json();
    let gets: Vec<Vec<&Value>> = held
        .as_array()
        .unwrap()
        .iter()
        .map(|attached| vec![&attached["entry"]["target"], &attached["override"]])
        .collect();
    assert_eq!(
        gets,
        [
            [&json!("GATEWAY_URL"), &json!(null)],
            [&json!("EDITOR"), &json!(null)]
        ]
    );

    let gateway = id(&gateway);
    let mut change = variable("GATEWAY_URL", "https://mine", false);
    change["template"] = json!(false);
    change["executable"] = json!(false);
    let changed = bob
        .call("PUT", &format!("/api/setup/{gateway}"), Some(change))
        .await;
    assert_eq!(changed.status, StatusCode::NOT_FOUND, "{}", changed.text());
    let attached = bob
        .call("PUT", &format!("/api/sandboxes/main/setup/{gateway}"), None)
        .await;
    refused(
        &attached,
        StatusCode::BAD_REQUEST,
        "invalid: variable GATEWAY_URL is required in every sandbox",
    );
}

#[tokio::test]
async fn a_shared_secret_never_shows_a_glimpse_to_members() {
    let corral = Corral::new();
    let mut root = corral.signed_in("root", Role::Admin).await;
    let mut bob = corral.signed_in("bob", Role::Member).await;
    let token = created(
        &root
            .post(
                SHARED_SETUP,
                shared_variable("GATEWAY_TOKEN", "gwt_s3cr3t-0123456789Xz", "required", true),
            )
            .await,
    );
    let key = created(
        &root
            .post(
                SHARED_SETUP,
                shared_variable("OPENAI_API_KEY", "sk-s3cr3t-0123456789Qw", "offered", true),
            )
            .await,
    );
    bob.post("/api/sandboxes", json!({ "name": "main" })).await;

    let replies = [
        bob.get("/api/setup").await,
        bob.get(&format!("/api/setup/{}", id(&token))).await,
        bob.get(&format!("/api/setup/{}", id(&key))).await,
        bob.get("/api/sandboxes/main/setup").await,
    ];
    for reply in &replies {
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
        for glimpse in [
            "s3cr3t", "gwt_", "sk-s", "Xz", "Qw", "length", "head", "tail",
        ] {
            assert!(!reply.text().contains(glimpse), "{}", reply.text());
        }
    }
    let listed = replies[0].json();
    let shown: Vec<&Value> = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| &entry["value"]["shown"])
        .collect();
    assert_eq!(shown, [&json!("hidden"), &json!("hidden")]);
    assert_eq!(replies[3].json()[0]["entry"]["value"]["shown"], "hidden");

    for (entry, length, head, tail) in [(&token, 23, "gwt_", "Xz"), (&key, 22, "sk-s", "Qw")] {
        let seen = root
            .get(&format!("{SHARED_SETUP}/{}", id(entry)))
            .await
            .json();
        assert_eq!(
            fields(&seen["value"], &["shown", "length", "head", "tail"]),
            [&json!("masked"), &json!(length), &json!(head), &json!(tail)]
        );
    }
}

#[tokio::test]
async fn an_offered_entry_lists_who_shadows_it_and_how_many_attached() {
    let corral = Corral::new();
    let mut root = corral.signed_in("root", Role::Admin).await;
    let mut bob = corral.signed_in("bob", Role::Member).await;
    bob.post("/api/sandboxes", json!({ "name": "main" })).await;
    let mut offered = shared_variable("OPENAI_BASE_URL", "https://llm/v1", "offered", false);
    offered["applyToNew"] = json!(false);
    let offered = created(&root.post(SHARED_SETUP, offered).await);

    let attached = bob
        .call(
            "PUT",
            &format!("/api/sandboxes/main/setup/{}", id(&offered)),
            None,
        )
        .await;
    assert_eq!(
        attached.status,
        StatusCode::NO_CONTENT,
        "{}",
        attached.text()
    );
    let own = created(
        &bob.post(
            "/api/setup",
            variable("OPENAI_BASE_URL", "https://own", false),
        )
        .await,
    );
    assert_eq!(
        fields(&own, &["shadows", "shadowed"]),
        [&json!(true), &json!(false)]
    );
    let seen = bob
        .get(&format!("/api/setup/{}", id(&offered)))
        .await
        .json();
    assert_eq!(
        fields(&seen, &["shared", "shadows", "shadowed"]),
        [&json!("offered"), &json!(false), &json!(true)]
    );

    let listed = root.get(SHARED_SETUP).await.json();
    assert_eq!(
        fields(&listed[0], &["target", "attached", "shadowedBy"]),
        [&json!("OPENAI_BASE_URL"), &json!(1), &json!(["bob"])]
    );
}

#[tokio::test]
async fn making_required_what_members_set_is_refused_naming_them() {
    let corral = Corral::new();
    let mut root = corral.signed_in("root", Role::Admin).await;
    for handle in ["bob", "alice"] {
        let mut member = corral.signed_in(handle, Role::Member).await;
        let theirs = variable("ANTHROPIC_BASE_URL", "https://theirs", false);
        created(&member.post("/api/setup", theirs).await);
    }

    let required = shared_variable("ANTHROPIC_BASE_URL", "https://llm", "required", false);
    refused(
        &root.post(SHARED_SETUP, required).await,
        StatusCode::CONFLICT,
        "conflict: variable ANTHROPIC_BASE_URL is also set by alice and bob; \
         ask them to rename theirs first",
    );
    assert_eq!(root.get(SHARED_SETUP).await.json(), json!([]));
}

#[tokio::test]
async fn shared_entries_change_and_delete() {
    let corral = Corral::new();
    let mut root = corral.signed_in("root", Role::Admin).await;
    let mut bob = corral.signed_in("bob", Role::Member).await;
    let offered = created(
        &root
            .post(
                SHARED_SETUP,
                shared_variable("OPENAI_BASE_URL", "https://llm/v1", "offered", false),
            )
            .await,
    );
    let uri = format!("{SHARED_SETUP}/{}", id(&offered));
    let own = created(
        &bob.post(
            "/api/setup",
            variable("OPENAI_BASE_URL", "https://own", false),
        )
        .await,
    );
    let mut change = shared_variable("OPENAI_BASE_URL", "https://llm/v2", "required", false);
    change["template"] = json!(false);
    change["executable"] = json!(false);
    refused(
        &root.call("PUT", &uri, Some(change.clone())).await,
        StatusCode::CONFLICT,
        "conflict: variable OPENAI_BASE_URL is also set by bob; ask them to rename theirs first",
    );
    let removed = bob
        .call("DELETE", &format!("/api/setup/{}", id(&own)), None)
        .await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT, "{}", removed.text());

    let changed = root.call("PUT", &uri, Some(change)).await;
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.text());
    assert_eq!(
        fields(&changed.json(), &["shared", "id"]),
        [&json!("required"), &offered["id"]]
    );
    assert_eq!(changed.json()["value"]["text"], "https://llm/v2");
    assert_eq!(root.get(&uri).await.json()["shared"], "required");
    assert_eq!(bob.get("/api/setup").await.json()[0]["shared"], "required");

    let deleted = root.call("DELETE", &uri, None).await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT, "{}", deleted.text());
    assert_eq!(root.get(&uri).await.status, StatusCode::NOT_FOUND);
    assert_eq!(bob.get("/api/setup").await.json(), json!([]));
}
