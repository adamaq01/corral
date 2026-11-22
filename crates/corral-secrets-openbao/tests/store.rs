mod support;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use corral_core::Error;
use corral_core::contract::secrets;
use corral_core::domain::{CredentialName, UserId};
use corral_core::fakes::MemoryUserRepo;
use corral_core::ports::{Env, SecretStore};
use corral_secrets_openbao::{OpenBaoAuth, OpenBaoConfig, OpenBaoStore};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};
use support::{MOUNT, OpenBao, PREFIX};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
use uuid::Uuid;
use vaultrs::api::auth::kubernetes::requests::{
    ConfigureKubernetesAuthRequest, CreateKubernetesRoleRequest,
};
use vaultrs::auth::kubernetes;
use vaultrs::{kv2, sys};

fn user() -> UserId {
    UserId(Uuid::new_v4())
}

fn name(value: &str) -> CredentialName {
    CredentialName::parse(value).unwrap()
}

fn secret(value: &str) -> SecretString {
    SecretString::from(value)
}

fn exposed(env: &Env) -> Vec<String> {
    env.iter()
        .map(|(name, value)| format!("{name}={}", value.expose_secret()))
        .collect()
}

fn path(user: UserId) -> String {
    format!("{PREFIX}/{}", user.0)
}

#[tokio::test]
async fn meets_the_secret_store_contract() {
    let bao = OpenBao::start().await;
    let store = OpenBaoStore::connect(bao.root_config()).await.unwrap();
    secrets::secret_store(&MemoryUserRepo::default(), &store).await;
}

#[tokio::test]
async fn each_user_owns_one_secret_holding_every_credential() {
    let bao = OpenBao::start().await;
    let mut config = bao.root_config();
    config.mount = format!("{MOUNT}/");
    config.path_prefix = format!("/{PREFIX}/");
    let store = OpenBaoStore::connect(config).await.unwrap();
    let adam = user();
    store.set(adam, &name("A_KEY"), secret("a")).await.unwrap();
    store.set(adam, &name("B_KEY"), secret("b")).await.unwrap();

    let stored: BTreeMap<String, String> = kv2::read(&bao.root, MOUNT, &path(adam)).await.unwrap();
    assert_eq!(
        stored,
        BTreeMap::from([("A_KEY".into(), "a".into()), ("B_KEY".into(), "b".into())])
    );
}

#[tokio::test]
async fn concurrent_writes_for_one_user_lose_nothing() {
    let bao = OpenBao::start().await;
    let store = Arc::new(OpenBaoStore::connect(bao.root_config()).await.unwrap());
    let adam = user();
    let mut writers = JoinSet::new();
    for i in 0..8 {
        let store = Arc::clone(&store);
        writers.spawn(async move {
            store
                .set(adam, &name(&format!("KEY_{i}")), secret(&format!("v{i}")))
                .await
        });
    }
    while let Some(written) = writers.join_next().await {
        written.unwrap().unwrap();
    }

    let expected: Vec<_> = (0..8).map(|i| format!("KEY_{i}=v{i}")).collect();
    assert_eq!(exposed(&store.load(adam).await.unwrap()), expected);
}

#[tokio::test]
async fn overwritten_and_deleted_values_leave_no_readable_history() {
    let bao = OpenBao::start().await;
    let store = OpenBaoStore::connect(bao.root_config()).await.unwrap();
    let adam = user();
    store
        .set(adam, &name("A_KEY"), secret("first"))
        .await
        .unwrap();
    store
        .set(adam, &name("A_KEY"), secret("second"))
        .await
        .unwrap();
    store.set(adam, &name("B_KEY"), secret("b")).await.unwrap();
    store.remove(adam, &name("B_KEY")).await.unwrap();

    assert_eq!(
        readable_versions(&bao, adam).await,
        [BTreeMap::from([("A_KEY".into(), "second".into())])]
    );

    store.remove(adam, &name("A_KEY")).await.unwrap();
    assert_eq!(readable_versions(&bao, adam).await, []);
    let metadata = kv2::read_metadata(&bao.root, MOUNT, &path(adam)).await;
    assert!(metadata.is_err(), "{metadata:?}");
    assert!(store.load(adam).await.unwrap().is_empty());

    store.set(adam, &name("C_KEY"), secret("c")).await.unwrap();
    assert_eq!(exposed(&store.load(adam).await.unwrap()), ["C_KEY=c"]);
}

// Probes every version number the store could have written, not only those metadata lists.
async fn readable_versions(bao: &OpenBao, user: UserId) -> Vec<BTreeMap<String, String>> {
    let mut readable = Vec::new();
    for version in 1..=10 {
        if let Ok(data) = kv2::read_version(&bao.root, MOUNT, &path(user), version).await {
            readable.push(data);
        }
    }
    readable
}

#[tokio::test]
async fn a_soft_deleted_secret_reads_empty_and_takes_new_writes() {
    let bao = OpenBao::start().await;
    let store = OpenBaoStore::connect(bao.root_config()).await.unwrap();
    let adam = user();
    store.set(adam, &name("A_KEY"), secret("a")).await.unwrap();
    kv2::delete_latest(&bao.root, MOUNT, &path(adam))
        .await
        .unwrap();

    assert!(store.load(adam).await.unwrap().is_empty());
    store.set(adam, &name("B_KEY"), secret("b")).await.unwrap();
    assert_eq!(exposed(&store.load(adam).await.unwrap()), ["B_KEY=b"]);
}

#[tokio::test]
async fn malformed_secrets_fail_without_quoting_values() {
    let bao = OpenBao::start().await;
    let store = OpenBaoStore::connect(bao.root_config()).await.unwrap();
    let adam = user();
    for (data, culprit) in [
        (json!({"A_KEY": "sk-quoted-1", "B_KEY": 42}), "B_KEY"),
        (
            json!({"A_KEY": "sk-quoted-1", "lower": "sk-quoted-2"}),
            "lower",
        ),
    ] {
        kv2::set(&bao.root, MOUNT, &path(adam), &data)
            .await
            .unwrap();
        match store.load(adam).await {
            Err(error @ Error::Internal { .. }) => {
                let message = error.to_string();
                assert!(
                    message.contains(culprit) && !message.contains("sk-quoted"),
                    "{message}"
                );
            }
            other => panic!("expected a secrets error, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn connect_rejects_a_bad_address_or_token() {
    let bao = OpenBao::start().await;
    let mut config = bao.root_config();
    config.address = "not a url".into();
    assert!(matches!(
        OpenBaoStore::connect(config.clone()).await,
        Err(Error::Invalid(_))
    ));
    config.address = "http://127.0.0.1:1".into();
    match OpenBaoStore::connect(config).await {
        Err(error @ Error::Internal { .. }) => {
            let message = error.to_string();
            assert!(message.contains("Connection refused"), "{message}");
        }
        Err(other) => panic!("expected a secrets error, got {other:?}"),
        Ok(_) => panic!("connected to a closed port"),
    }

    let wrong = bao.config(OpenBaoAuth::Token(secret("s.wrong-token-value")));
    match OpenBaoStore::connect(wrong).await {
        Err(error @ Error::Internal { .. }) => {
            let message = error.to_string();
            assert!(
                message.contains("403") && !message.contains("wrong-token-value"),
                "{message}"
            );
        }
        Err(other) => panic!("expected a secrets error, got {other:?}"),
        Ok(_) => panic!("connected with a wrong token"),
    }
}

#[test]
fn config_debug_hides_the_token() {
    let config = OpenBaoConfig {
        address: "http://bao:8200".into(),
        mount: MOUNT.into(),
        path_prefix: PREFIX.into(),
        auth: OpenBaoAuth::Token(secret("s.very-secret")),
    };
    let debug = format!("{config:?}");
    assert!(
        debug.contains("http://bao:8200") && !debug.contains("very-secret"),
        "{debug}"
    );
}

#[tokio::test]
async fn kubernetes_auth_logs_in_again_with_the_current_jwt_once_its_token_expires() {
    let reviewed = Arc::new(Mutex::new(Vec::new()));
    let review_port = serve_token_reviews(Arc::clone(&reviewed)).await;
    let bao = OpenBao::start().await;
    enable_kubernetes_auth(&bao, review_port, "3s").await;
    let jwt_path = std::env::temp_dir().join(format!("corral-openbao-{}.jwt", Uuid::new_v4()));
    let (first, second) = (service_account_jwt(), service_account_jwt());
    std::fs::write(&jwt_path, format!("{first}\n")).unwrap();

    let store = OpenBaoStore::connect(bao.config(OpenBaoAuth::Kubernetes {
        role: "corral".into(),
        jwt_path: jwt_path.clone(),
    }))
    .await
    .unwrap();
    let adam = user();
    store.set(adam, &name("A_KEY"), secret("a")).await.unwrap();

    std::fs::write(&jwt_path, &second).unwrap();
    tokio::time::sleep(Duration::from_secs(4)).await;
    store.set(adam, &name("B_KEY"), secret("b")).await.unwrap();

    assert_eq!(
        exposed(&store.load(adam).await.unwrap()),
        ["A_KEY=a", "B_KEY=b"]
    );
    assert_eq!(*reviewed.lock().unwrap(), [first, second]);

    store.remove(adam, &name("A_KEY")).await.unwrap();
    store.remove(adam, &name("B_KEY")).await.unwrap();
    assert!(store.load(adam).await.unwrap().is_empty());
    std::fs::remove_file(jwt_path).unwrap();
}

async fn enable_kubernetes_auth(bao: &OpenBao, review_port: u16, token_ttl: &str) {
    sys::auth::enable(&bao.root, "kubernetes", "kubernetes", None)
        .await
        .unwrap();
    kubernetes::configure(
        &bao.root,
        "kubernetes",
        &format!("http://host.docker.internal:{review_port}"),
        Some(
            ConfigureKubernetesAuthRequest::builder()
                .disable_local_ca_jwt(true)
                .kubernetes_ca_cert(include_str!("fixtures/kubernetes-ca.pem")),
        ),
    )
    .await
    .unwrap();
    let policy = format!(
        r#"path "{MOUNT}/data/{PREFIX}/*" {{ capabilities = ["create", "read", "update"] }}
path "{MOUNT}/metadata/{PREFIX}/*" {{ capabilities = ["read", "create", "update", "delete"] }}"#
    );
    sys::policy::set(&bao.root, "corral", &policy)
        .await
        .unwrap();
    kubernetes::role::create(
        &bao.root,
        "kubernetes",
        "corral",
        Some(
            CreateKubernetesRoleRequest::builder()
                .bound_service_account_names(vec!["corral".to_owned()])
                .bound_service_account_namespaces(vec!["corral".to_owned()])
                .token_policies(vec!["corral".to_owned()])
                .token_ttl(token_ttl),
        ),
    )
    .await
    .unwrap();
}

fn service_account_jwt() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let part = |value: Value| URL_SAFE_NO_PAD.encode(value.to_string());
    let claims = json!({
        "iss": "kubernetes/serviceaccount",
        "sub": "system:serviceaccount:corral:corral",
        "jti": Uuid::new_v4().to_string(),
        "iat": now - 60,
        "nbf": now - 60,
        "exp": now + 3600,
        "kubernetes.io/serviceaccount/namespace": "corral",
        "kubernetes.io/serviceaccount/service-account.name": "corral",
        "kubernetes.io/serviceaccount/service-account.uid": "uid-1",
    });
    format!(
        "{}.{}.{}",
        part(json!({"alg": "RS256", "typ": "JWT"})),
        part(claims),
        URL_SAFE_NO_PAD.encode("unsigned")
    )
}

async fn serve_token_reviews(reviewed: Arc<Mutex<Vec<String>>>) -> u16 {
    let listener = TcpListener::bind("0.0.0.0:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let (socket, _) = listener.accept().await.unwrap();
            let mut socket = BufReader::new(socket);
            let review = read_json_body(&mut socket).await;
            let token = review["spec"]["token"].as_str().unwrap().to_owned();
            reviewed.lock().unwrap().push(token);
            let reply = json!({
                "apiVersion": "authentication.k8s.io/v1",
                "kind": "TokenReview",
                "status": {
                    "authenticated": true,
                    "user": {"username": "system:serviceaccount:corral:corral", "uid": "uid-1"},
                },
            })
            .to_string();
            let response = format!(
                "HTTP/1.1 201 Created\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply}",
                reply.len()
            );
            socket
                .get_mut()
                .write_all(response.as_bytes())
                .await
                .unwrap();
        }
    });
    port
}

async fn read_json_body(socket: &mut BufReader<TcpStream>) -> Value {
    let mut length = 0;
    loop {
        let mut line = String::new();
        if socket.read_line(&mut line).await.unwrap() <= 2 {
            break;
        }
        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            length = value.trim().parse().unwrap();
        }
    }
    let mut body = vec![0; length];
    socket.read_exact(&mut body).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}
