mod support;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use corral_core::Error;
use corral_core::contract::secrets;
use corral_core::domain::{CredentialName, UserId};
use corral_core::fakes::exposed;
use corral_core::ports::SecretStore;
use corral_secrets_vault::{VaultAuth, VaultConfig, VaultStore};
use secrecy::SecretString;
use serde_json::{Value, json};
use support::{KvServer, MOUNT, PREFIX, Server};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
use uuid::Uuid;
use vaultrs::api::auth::kubernetes::requests::{
    ConfigureKubernetesAuthRequest, CreateKubernetesRoleRequest,
};
use vaultrs::auth::kubernetes;
use vaultrs::{kv2, sys};

against_each_server!(
    meets_the_secret_store_contract,
    each_user_owns_one_secret_holding_every_credential,
    concurrent_writes_for_one_user_lose_nothing,
    overwritten_and_deleted_values_leave_no_readable_history,
    a_soft_deleted_secret_reads_empty_and_takes_new_writes,
    malformed_secrets_fail_without_quoting_values,
    connect_rejects_a_bad_address_or_token,
    kubernetes_auth_logs_in_again_with_the_current_jwt_once_its_token_expires,
);

fn user() -> UserId {
    UserId(Uuid::new_v4())
}

fn name(value: &str) -> CredentialName {
    CredentialName::parse(value).unwrap()
}

fn secret(value: &str) -> SecretString {
    SecretString::from(value)
}

fn path(user: UserId) -> String {
    format!("{PREFIX}/{}", user.0)
}

async fn meets_the_secret_store_contract(server: Server) {
    let kv = KvServer::start(server).await;
    let store = VaultStore::connect(kv.root_config()).await.unwrap();
    secrets::secret_store(&store).await;
}

async fn each_user_owns_one_secret_holding_every_credential(server: Server) {
    let kv = KvServer::start(server).await;
    let mut config = kv.root_config();
    config.mount = format!("{MOUNT}/");
    config.path_prefix = format!("/{PREFIX}/");
    let store = VaultStore::connect(config).await.unwrap();
    let adam = user();
    store.set(adam, &name("A_KEY"), secret("a")).await.unwrap();
    store.set(adam, &name("B_KEY"), secret("b")).await.unwrap();

    let stored: BTreeMap<String, String> = kv2::read(&kv.root, MOUNT, &path(adam)).await.unwrap();
    assert_eq!(
        stored,
        BTreeMap::from([("A_KEY".into(), "a".into()), ("B_KEY".into(), "b".into())])
    );
}

async fn concurrent_writes_for_one_user_lose_nothing(server: Server) {
    let kv = KvServer::start(server).await;
    let store = Arc::new(VaultStore::connect(kv.root_config()).await.unwrap());
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

async fn overwritten_and_deleted_values_leave_no_readable_history(server: Server) {
    let kv = KvServer::start(server).await;
    let store = VaultStore::connect(kv.root_config()).await.unwrap();
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
        readable_versions(&kv, adam).await,
        [BTreeMap::from([("A_KEY".into(), "second".into())])]
    );

    store.remove(adam, &name("A_KEY")).await.unwrap();
    assert_eq!(readable_versions(&kv, adam).await, [BTreeMap::new()]);
    assert!(store.load(adam).await.unwrap().is_empty());

    store.set(adam, &name("C_KEY"), secret("c")).await.unwrap();
    assert_eq!(exposed(&store.load(adam).await.unwrap()), ["C_KEY=c"]);
}

// Probes every version number the store could have written, not only those metadata lists.
async fn readable_versions(kv: &KvServer, user: UserId) -> Vec<BTreeMap<String, String>> {
    let mut readable = Vec::new();
    for version in 1..=10 {
        if let Ok(data) = kv2::read_version(&kv.root, MOUNT, &path(user), version).await {
            readable.push(data);
        }
    }
    readable
}

async fn a_soft_deleted_secret_reads_empty_and_takes_new_writes(server: Server) {
    let kv = KvServer::start(server).await;
    let store = VaultStore::connect(kv.root_config()).await.unwrap();
    let adam = user();
    store.set(adam, &name("A_KEY"), secret("a")).await.unwrap();
    kv2::delete_latest(&kv.root, MOUNT, &path(adam))
        .await
        .unwrap();

    assert!(store.load(adam).await.unwrap().is_empty());
    store.set(adam, &name("B_KEY"), secret("b")).await.unwrap();
    assert_eq!(exposed(&store.load(adam).await.unwrap()), ["B_KEY=b"]);
}

async fn malformed_secrets_fail_without_quoting_values(server: Server) {
    let kv = KvServer::start(server).await;
    let store = VaultStore::connect(kv.root_config()).await.unwrap();
    let adam = user();
    for (data, culprit) in [
        (json!({"A_KEY": "sk-quoted-1", "B_KEY": 42}), "B_KEY"),
        (
            json!({"A_KEY": "sk-quoted-1", "lower": "sk-quoted-2"}),
            "lower",
        ),
    ] {
        kv2::set(&kv.root, MOUNT, &path(adam), &data).await.unwrap();
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

async fn connect_rejects_a_bad_address_or_token(server: Server) {
    let kv = KvServer::start(server).await;
    let mut config = kv.root_config();
    config.address = "not a url".into();
    assert!(matches!(
        VaultStore::connect(config.clone()).await,
        Err(Error::Invalid(_))
    ));
    config.address = "http://127.0.0.1:1".into();
    match VaultStore::connect(config).await {
        Err(error @ Error::Internal { .. }) => {
            let message = error.to_string();
            assert!(message.contains("Connection refused"), "{message}");
        }
        Err(other) => panic!("expected a secrets error, got {other:?}"),
        Ok(_) => panic!("connected to a closed port"),
    }

    let wrong = kv.config(VaultAuth::Token {
        token: secret("s.wrong-token-value"),
    });
    match VaultStore::connect(wrong).await {
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
    let config = VaultConfig {
        address: "http://vault:8200".into(),
        mount: MOUNT.into(),
        path_prefix: PREFIX.into(),
        auth: VaultAuth::Token {
            token: secret("s.very-secret"),
        },
    };
    let debug = format!("{config:?}");
    assert!(
        debug.contains("http://vault:8200") && !debug.contains("very-secret"),
        "{debug}"
    );
}

async fn kubernetes_auth_logs_in_again_with_the_current_jwt_once_its_token_expires(server: Server) {
    let reviewed = Arc::new(Mutex::new(Vec::new()));
    let review_port = serve_token_reviews(Arc::clone(&reviewed)).await;
    let kv = KvServer::start(server).await;
    enable_kubernetes_auth(&kv, review_port, "3s").await;
    let jwt_path = std::env::temp_dir().join(format!("corral-vault-{}.jwt", Uuid::new_v4()));
    let (first, second) = (service_account_jwt(), service_account_jwt());
    std::fs::write(&jwt_path, format!("{first}\n")).unwrap();

    let store = VaultStore::connect(kv.config(VaultAuth::Kubernetes {
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

async fn enable_kubernetes_auth(kv: &KvServer, review_port: u16, token_ttl: &str) {
    sys::auth::enable(&kv.root, "kubernetes", "kubernetes", None)
        .await
        .unwrap();
    kubernetes::configure(
        &kv.root,
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
path "{MOUNT}/metadata/{PREFIX}/*" {{ capabilities = ["read", "create", "update"] }}"#
    );
    sys::policy::set(&kv.root, "corral", &policy).await.unwrap();
    kubernetes::role::create(
        &kv.root,
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
