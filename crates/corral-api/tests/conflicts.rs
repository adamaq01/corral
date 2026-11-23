use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode, header};
use corral_api::testing::trusting_provider;
use corral_api::{Api, Key, SshEndpoint, router};
use corral_core::app::{Accounts, Credentials, Keys, Limits, Sandboxes};
use corral_core::fakes::{ADAM_FINGERPRINT, ADAM_KEY, FakeEngine, MemorySecretStore, catalog};
use corral_sql::testing::Databases;
use corral_store_sql::Store;
use serde_json::{Value, json};
use tower::ServiceExt;

async fn app(databases: &Databases) -> Router {
    let store = Arc::new(Store::open(databases.pool().await).await.unwrap());
    let credentials = Credentials::new(Arc::new(MemorySecretStore::default()), store.clone());
    let sandboxes = Sandboxes::new(
        store.clone(),
        store.clone(),
        Arc::new(FakeEngine::default()),
        credentials.clone(),
        catalog(),
        Limits {
            max_sandboxes_per_user: 5,
            wake_deadline: Duration::from_secs(1),
        },
    );
    let api = Api {
        accounts: Accounts::new(
            store.clone(),
            store.clone(),
            sandboxes.clone(),
            credentials.clone(),
            Vec::new(),
        ),
        keys: Keys::new(store),
        credentials,
        sandboxes,
        ssh: SshEndpoint {
            host: "corral.example.com".to_owned(),
            port: 22,
        },
        session_key: Key::generate(),
        secure_cookies: false,
        trust_forwarded_for: false,
    };
    router(api, vec![trusting_provider("test")])
}

struct Reply {
    status: StatusCode,
    cookies: String,
    body: Value,
}

async fn post(app: &Router, uri: &str, cookies: &str, body: Value) -> Reply {
    let mut request = Request::post(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, cookies)
        .body(Body::from(body.to_string()))
        .unwrap();
    request.extensions_mut().insert(ConnectInfo(
        "192.0.2.10:50000".parse::<SocketAddr>().unwrap(),
    ));
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let cookies: Vec<&str> = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok()?.split(';').next())
        .collect();
    let cookies = cookies.join("; ");
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    Reply {
        status,
        cookies,
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    }
}

async fn sign_in(app: &Router, username: &str) -> String {
    let reply = post(
        app,
        "/auth/test/login",
        "",
        json!({ "subject": username, "username": username, "role": "member" }),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    reply.cookies
}

async fn duplicates_name_the_thing_not_the_table(databases: Databases) {
    let app = app(&databases).await;
    let adam = sign_in(&app, "adam").await;
    let eve = sign_in(&app, "eve").await;

    let main = json!({ "name": "main" });
    let created = post(&app, "/api/sandboxes", &adam, main.clone()).await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    let duplicate = post(&app, "/api/sandboxes", &adam, main).await;
    assert_eq!(duplicate.status, StatusCode::CONFLICT);
    assert_eq!(
        duplicate.body,
        json!({ "error": "conflict: sandbox main already exists" })
    );

    let key = |label: &str| json!({ "label": label, "publicKey": ADAM_KEY });
    let added = post(&app, "/api/keys", &adam, key("laptop")).await;
    assert_eq!(added.status, StatusCode::CREATED, "{}", added.body);
    for (owner, label) in [(&adam, "again"), (&eve, "stolen")] {
        let duplicate = post(&app, "/api/keys", owner, key(label)).await;
        assert_eq!(duplicate.status, StatusCode::CONFLICT);
        assert_eq!(
            duplicate.body,
            json!({ "error": format!("conflict: key {ADAM_FINGERPRINT} already registered") })
        );
    }
}

#[tokio::test]
async fn sqlite_duplicates_name_the_thing_not_the_table() {
    duplicates_name_the_thing_not_the_table(Databases::sqlite()).await;
}

#[tokio::test]
async fn postgres_duplicates_name_the_thing_not_the_table() {
    duplicates_name_the_thing_not_the_table(Databases::postgres().await).await;
}
