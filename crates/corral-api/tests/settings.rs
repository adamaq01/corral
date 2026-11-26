use axum::Router;
use axum::http::StatusCode;
use corral_api::router;
use corral_api::testing::{Browser, Reply, api, trusting_provider};
use corral_core::app::{CatalogChange, Configured, Registry, Source};
use corral_core::domain::Role;
use corral_core::fakes::{EngineCall, MAX_SANDBOXES, PROVIDER, World, image_option};
use corral_core::ports::SettingsRepo;
use serde_json::{Value, json};

struct Corral {
    app: Router,
    world: World,
}

impl Corral {
    fn new() -> Self {
        Self::over(World::new())
    }

    fn over(world: World) -> Self {
        Self {
            app: router(api(&world), vec![trusting_provider(PROVIDER)]),
            world,
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

async fn put(browser: &mut Browser, uri: &str, body: Value) -> Reply {
    browser.call("PUT", uri, Some(body)).await
}

fn refused(reply: &Reply, status: StatusCode, mentioning: &[&str]) {
    assert_eq!(reply.status, status, "{}", reply.text());
    let error = reply.json()["error"].as_str().unwrap().to_owned();
    for part in mentioning {
        assert!(error.contains(part), "{error:?} does not mention {part:?}");
    }
}

fn image(key: &str, enabled: bool) -> Value {
    json!({
        "key": key,
        "label": format!("{key} image"),
        "reference": format!("ghcr.io/acme/{key}:1"),
        "description": format!("What {key} has"),
        "enabled": enabled,
    })
}

fn size(key: &str, cpu: (&str, &str), memory: (&str, &str), home: &str) -> Value {
    json!({
        "key": key,
        "label": format!("{key} size"),
        "cpuRequest": cpu.0,
        "cpuLimit": cpu.1,
        "memoryRequest": memory.0,
        "memoryLimit": memory.1,
        "homeSize": home,
        "enabled": true,
    })
}

fn owned(mut entry: Value, source: &str) -> Value {
    entry["source"] = json!(source);
    entry
}

fn small() -> Value {
    size("small", ("500m", "1"), ("512Mi", "2Gi"), "10Gi")
}

fn large() -> Value {
    size("large", ("2", "4"), ("4Gi", "8Gi"), "50Gi")
}

fn sizes_of(catalog: &Value) -> Vec<&str> {
    catalog["sizes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|size| size["key"].as_str().unwrap())
        .collect()
}

fn user<'a>(users: &'a Value, handle: &str) -> &'a Value {
    users
        .as_array()
        .unwrap()
        .iter()
        .find(|user| user["handle"] == handle)
        .unwrap()
}

#[tokio::test]
async fn an_admin_changes_the_catalog_and_members_see_what_is_offered() {
    let corral = Corral::new();
    let mut root = corral.signed_in("root", Role::Admin).await;
    let mut adam = corral.signed_in("adam", Role::Member).await;
    let change = json!({
        "images": [image("base", true), image("tools", false)],
        "sizes": [
            size("small", ("0.5", "1"), ("512Mi", "2048Mi"), "10Gi"),
            large(),
        ],
        "defaults": { "image": "base", "size": "small" },
        "namedImages": { "registries": ["ghcr.io/acme"], "maxSize": "2Gi" },
    });
    let saved = put(&mut root, "/api/admin/settings/catalog", change).await;
    assert_eq!(saved.status, StatusCode::NO_CONTENT, "{}", saved.text());

    assert_eq!(
        root.get("/api/admin/settings/catalog").await.json(),
        json!({
            "images": [
                owned(image("base", true), "database"),
                owned(image("tools", false), "database"),
            ],
            "sizes": [owned(small(), "database"), owned(large(), "database")],
            "namedImages": { "registries": ["ghcr.io/acme"], "maxSize": "2Gi" },
            "namedImagesSource": "database",
            "defaultImage": "base",
            "defaultSize": "small",
            "defaultsSource": "database",
            "defaultsFellBack": false,
            "shadowedImages": [],
            "shadowedSizes": [],
        })
    );
    let catalog = adam.get("/api/catalog").await.json();
    assert_eq!(
        catalog["images"],
        json!([{ "key": "base", "label": "base image" }])
    );
    assert_eq!(sizes_of(&catalog), ["small", "large"]);
    assert_eq!(
        catalog["namedImages"],
        json!({ "registries": ["ghcr.io/acme"], "maxBytes": 2u64 << 30 })
    );
    let tools = adam
        .post(
            "/api/sandboxes",
            json!({ "name": "main", "image": "tools" }),
        )
        .await;
    refused(&tools, StatusCode::BAD_REQUEST, &["tools"]);

    let member = put(
        &mut adam,
        "/api/admin/settings/catalog",
        json!({ "images": [], "sizes": [] }),
    )
    .await;
    assert_eq!(member.status, StatusCode::FORBIDDEN);
    let bad_cpu = put(
        &mut root,
        "/api/admin/settings/catalog",
        json!({ "images": [image("base", true)], "sizes": [size("small", ("lots", "1"), ("1Gi", "2Gi"), "10Gi")] }),
    )
    .await;
    refused(&bad_cpu, StatusCode::BAD_REQUEST, &["small", "lots"]);
}

#[tokio::test]
async fn what_config_sets_is_locked_in_the_api() {
    let corral = Corral::over(World::with_configured(Configured {
        images: vec![image_option("ubuntu", "ghcr.io/adamaq01/corral-sandbox")],
        max_running: Some(3),
        registries: vec![Registry {
            registry: "ghcr.io".to_owned(),
            username: None,
            password_env: None,
            plain_http: false,
            source: Source::Config,
        }],
        ..Configured::default()
    }));
    let mut root = corral.signed_in("root", Role::Admin).await;

    let catalog = root.get("/api/admin/settings/catalog").await.json();
    let sources: Vec<(&Value, &Value)> = catalog["images"]
        .as_array()
        .unwrap()
        .iter()
        .map(|image| (&image["key"], &image["source"]))
        .collect();
    assert_eq!(
        sources,
        [
            (&json!("ubuntu"), &json!("config")),
            (&json!("base"), &json!("database")),
            (&json!("tools"), &json!("database")),
        ]
    );
    let taken = put(
        &mut root,
        "/api/admin/settings/catalog",
        json!({ "images": [image("base", true), image("ubuntu", true)], "sizes": [small()] }),
    )
    .await;
    refused(
        &taken,
        StatusCode::BAD_REQUEST,
        &["ubuntu", "set in config"],
    );

    let limits = root.get("/api/admin/settings/limits").await.json();
    assert_eq!(
        limits,
        json!({
            "maxSandboxesPerMember": { "value": MAX_SANDBOXES, "source": "database" },
            "maxRunning": { "value": 3, "source": "config" },
        })
    );
    let running = put(
        &mut root,
        "/api/admin/settings/limits",
        json!({ "maxRunning": 9 }),
    )
    .await;
    refused(&running, StatusCode::BAD_REQUEST, &["set in config"]);
    let per_member = put(
        &mut root,
        "/api/admin/settings/limits",
        json!({ "maxSandboxesPerMember": 2 }),
    )
    .await;
    assert_eq!(per_member.status, StatusCode::NO_CONTENT);
    assert_eq!(
        root.get("/api/admin/settings/limits").await.json(),
        json!({
            "maxSandboxesPerMember": { "value": 2, "source": "database" },
            "maxRunning": { "value": 3, "source": "config" },
        })
    );

    let registries = root.get("/api/admin/settings/registries").await.json();
    assert_eq!(registries["registries"][0]["source"], "config");
    let registry = json!([{
        "registry": "GHCR.io", "username": null, "passwordEnv": null, "plainHttp": false,
    }]);
    let taken = put(&mut root, "/api/admin/settings/registries", registry).await;
    refused(&taken, StatusCode::BAD_REQUEST, &["set in config"]);
}

#[tokio::test]
async fn the_builtin_catalog_answers_its_own_source() {
    let world = World::with_configured(Configured::default());
    world
        .settings_repo
        .set_catalog(&CatalogChange::default())
        .await
        .unwrap();
    let corral = Corral::over(world);
    let mut root = corral.signed_in("root", Role::Admin).await;

    let catalog = root.get("/api/admin/settings/catalog").await.json();
    let sources: Vec<(&Value, &Value)> = ["images", "sizes"]
        .iter()
        .flat_map(|kind| catalog[kind].as_array().unwrap())
        .map(|entry| (&entry["key"], &entry["source"]))
        .collect();
    assert_eq!(
        sources,
        [
            (&json!("ubuntu"), &json!("builtin")),
            (&json!("small"), &json!("builtin")),
        ]
    );
}

#[tokio::test]
async fn a_catalog_that_removes_a_used_size_is_refused_naming_the_sandbox() {
    let corral = Corral::new();
    let mut root = corral.signed_in("root", Role::Admin).await;
    let mut adam = corral.signed_in("adam", Role::Member).await;
    let created = adam.post("/api/sandboxes", json!({ "name": "main" })).await;
    assert_eq!(created.json()["size"], "small");

    let without_small = json!({
        "images": [image("base", true)],
        "sizes": [large()],
        "defaults": { "image": "base", "size": "large" },
    });
    let removed = put(&mut root, "/api/admin/settings/catalog", without_small).await;
    refused(&removed, StatusCode::BAD_REQUEST, &["small", "main.adam"]);
    let catalog = root.get("/api/admin/settings/catalog").await.json();
    assert_eq!(sizes_of(&catalog), ["small", "large"]);

    let mut disabled = small();
    disabled["enabled"] = json!(false);
    let disabling = json!({
        "images": [image("base", true)],
        "sizes": [disabled, large()],
        "defaults": { "image": "base", "size": "large" },
    });
    let saved = put(&mut root, "/api/admin/settings/catalog", disabling).await;
    assert_eq!(saved.status, StatusCode::NO_CONTENT, "{}", saved.text());
    assert_eq!(sizes_of(&adam.get("/api/catalog").await.json()), ["large"]);
}

#[tokio::test]
async fn limits_and_allowances_show_in_the_catalog_the_member_sees() {
    let corral = Corral::new();
    let mut root = corral.signed_in("root", Role::Admin).await;
    let mut adam = corral.signed_in("adam", Role::Member).await;
    let limits = json!({ "maxSandboxesPerMember": 2, "maxRunning": 6 });
    let saved = put(&mut root, "/api/admin/settings/limits", limits).await;
    assert_eq!(saved.status, StatusCode::NO_CONTENT, "{}", saved.text());
    assert_eq!(adam.get("/api/catalog").await.json()["maxSandboxes"], 2);

    let users = root.get("/api/admin/users").await.json();
    let listed = user(&users, "adam");
    assert_eq!(
        (&listed["provider"], &listed["allowance"]),
        (&json!(PROVIDER), &Value::Null)
    );
    let allowance = format!(
        "/api/admin/users/{}/allowance",
        listed["id"].as_str().unwrap()
    );
    let raised = put(&mut root, &allowance, json!({ "maxSandboxes": 4 })).await;
    assert_eq!(raised.status, StatusCode::OK, "{}", raised.text());
    let raised = raised.json();
    assert_eq!(
        (&raised["handle"], &raised["allowance"]),
        (&json!("adam"), &json!({ "maxSandboxes": 4 }))
    );
    assert_eq!(adam.get("/api/catalog").await.json()["maxSandboxes"], 4);
    let users = root.get("/api/admin/users").await.json();
    assert_eq!(
        user(&users, "adam")["allowance"],
        json!({ "maxSandboxes": 4 })
    );
    assert_eq!(user(&users, "root")["allowance"], Value::Null);

    let cleared = put(&mut root, &allowance, json!({ "maxSandboxes": null })).await;
    assert_eq!(cleared.json()["allowance"], Value::Null);
    assert_eq!(adam.get("/api/catalog").await.json()["maxSandboxes"], 2);

    let none = put(&mut root, &allowance, json!({ "maxSandboxes": 0 })).await;
    refused(&none, StatusCode::BAD_REQUEST, &["at least 1"]);
    let nobody = put(
        &mut root,
        "/api/admin/users/00000000-0000-0000-0000-000000000000/allowance",
        json!({ "maxSandboxes": 4 }),
    )
    .await;
    assert_eq!(nobody.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn registry_settings_say_whether_each_password_is_set() {
    let corral = Corral::new();
    let mut root = corral.signed_in("root", Role::Admin).await;
    let rows = json!([
        {
            "registry": "ghcr.io", "username": "corral",
            "passwordEnv": "CORRAL_TEST_PASSWORD", "plainHttp": false,
        },
        {
            "registry": "quay.io", "username": "corral",
            "passwordEnv": "CORRAL_UNSET_PASSWORD", "plainHttp": false,
        },
        { "registry": "registry.lan:5000", "username": null, "passwordEnv": null, "plainHttp": true },
    ]);
    let saved = put(&mut root, "/api/admin/settings/registries", rows.clone()).await;
    assert_eq!(saved.status, StatusCode::NO_CONTENT, "{}", saved.text());

    let expected: Vec<Value> = rows
        .as_array()
        .unwrap()
        .iter()
        .zip([true, false, false])
        .map(|(row, password_set)| {
            let mut row = owned(row.clone(), "database");
            row["passwordSet"] = json!(password_set);
            row
        })
        .collect();
    assert_eq!(
        root.get("/api/admin/settings/registries").await.json(),
        json!({ "registries": expected, "shadowed": [] })
    );

    let half = json!([{
        "registry": "ghcr.io", "username": "corral", "passwordEnv": null, "plainHttp": false,
    }]);
    let refused_half = put(&mut root, "/api/admin/settings/registries", half).await;
    refused(&refused_half, StatusCode::BAD_REQUEST, &["ghcr.io"]);
}

#[tokio::test]
async fn an_admin_wakes_a_sleeping_sandbox() {
    let corral = Corral::new();
    let mut root = corral.signed_in("root", Role::Admin).await;
    let mut eve = corral.signed_in("eve", Role::Member).await;
    eve.post("/api/sandboxes", json!({ "name": "main" })).await;
    eve.post("/api/sandboxes/main/stop", json!({})).await;
    assert_eq!(
        eve.get("/api/sandboxes/main").await.json()["state"],
        "sleeping"
    );

    let woken = root
        .post("/api/admin/sandboxes/eve/main/wake", json!({}))
        .await;
    assert_eq!(woken.status, StatusCode::OK, "{}", woken.text());
    let sandbox = woken.json();
    assert_eq!(
        (
            &sandbox["owner"]["handle"],
            &sandbox["name"],
            &sandbox["state"]
        ),
        (&json!("eve"), &json!("main"), &json!("running"))
    );
    assert_eq!(
        eve.get("/api/sandboxes/main").await.json()["state"],
        "running"
    );
    let missing = root
        .post("/api/admin/sandboxes/eve/spare/wake", json!({}))
        .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn admins_act_on_anyones_sandbox_and_members_cannot() {
    let corral = Corral::new();
    let mut root = corral.signed_in("root", Role::Admin).await;
    let mut eve = corral.signed_in("eve", Role::Member).await;
    let mut ada = corral.signed_in("ada", Role::Admin).await;
    eve.post("/api/sandboxes", json!({ "name": "main" })).await;
    ada.post("/api/sandboxes", json!({ "name": "main" })).await;
    let calls = corral.world.engine.calls();
    let Some(EngineCall::Create { id: eves, .. }) = calls.first() else {
        panic!("{calls:?}");
    };

    let stopped = root
        .post("/api/admin/sandboxes/eve/main/stop", json!({}))
        .await;
    assert_eq!(stopped.status, StatusCode::NO_CONTENT, "{}", stopped.text());
    assert!(
        corral
            .world
            .engine
            .calls()
            .iter()
            .any(|call| matches!(call, EngineCall::Stop(id) if id == eves))
    );
    assert_eq!(
        eve.get("/api/sandboxes/main").await.json()["state"],
        "sleeping"
    );
    let restarted = root
        .post("/api/admin/sandboxes/ada/main/restart", json!({}))
        .await;
    assert_eq!(
        restarted.status,
        StatusCode::NO_CONTENT,
        "{}",
        restarted.text()
    );

    for missing in [
        "/api/admin/sandboxes/nobody/main/stop",
        "/api/admin/sandboxes/eve/spare/stop",
    ] {
        let reply = root.post(missing, json!({})).await;
        assert_eq!(reply.status, StatusCode::NOT_FOUND, "{missing}");
    }
    let member = eve
        .post("/api/admin/sandboxes/ada/main/stop", json!({}))
        .await;
    assert_eq!(member.status, StatusCode::FORBIDDEN);
    let member = eve
        .call("DELETE", "/api/admin/sandboxes/ada/main", None)
        .await;
    assert_eq!(member.status, StatusCode::FORBIDDEN);

    let deleted = root
        .call("DELETE", "/api/admin/sandboxes/eve/main", None)
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT, "{}", deleted.text());
    let owners: Vec<Value> = root
        .get("/api/admin/sandboxes")
        .await
        .json()
        .as_array()
        .unwrap()
        .iter()
        .map(|sandbox| sandbox["owner"]["handle"].clone())
        .collect();
    assert_eq!(owners, [json!("ada")]);
}
