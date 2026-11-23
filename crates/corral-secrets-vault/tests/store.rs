mod support;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use corral_core::Error;
use corral_core::contract::secrets::{self, LegacyWriter};
use corral_core::domain::{SecretId, UserId};
use corral_core::fakes::exposed;
use corral_core::ports::SecretStore;
use corral_secrets_vault::{VaultAuth, VaultConfig, VaultStore};
use secrecy::{ExposeSecret, SecretSlice, SecretString};
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
    each_value_is_a_secret_of_its_own_beside_the_credentials_kept_before,
    concurrent_writes_for_one_owner_lose_nothing,
    an_overwritten_value_leaves_no_readable_history,
    a_soft_deleted_value_is_left_out,
    malformed_secrets_fail_without_quoting_values,
    connect_rejects_a_bad_address_or_token,
    kubernetes_auth_logs_in_again_with_the_current_jwt_once_its_token_expires,
);

/// Writes credentials as the store did before entries: one secret per owner, a map of names.
struct Legacy<'a>(&'a KvServer);

#[async_trait]
impl LegacyWriter for Legacy<'_> {
    async fn keep(&self, owner: UserId, name: &str, value: &str) {
        let path = owner_path(owner);
        let mut data: BTreeMap<String, String> = kv2::read(&self.0.root, MOUNT, &path)
            .await
            .unwrap_or_default();
        data.insert(name.to_owned(), value.to_owned());
        kv2::set(&self.0.root, MOUNT, &path, &data).await.unwrap();
    }
}

fn user() -> UserId {
    UserId(Uuid::new_v4())
}

fn bytes(value: &str) -> SecretSlice<u8> {
    SecretSlice::from(value.as_bytes().to_vec())
}

fn secret(value: &str) -> SecretString {
    SecretString::from(value)
}

fn owner_path(owner: UserId) -> String {
    format!("{PREFIX}/{}", owner.0)
}

fn path(owner: UserId, id: SecretId) -> String {
    format!("{PREFIX}/{}/{}", owner.0, id.0)
}

async fn read(store: &VaultStore, owner: UserId, ids: &[SecretId]) -> BTreeMap<SecretId, Vec<u8>> {
    store
        .get(owner, ids)
        .await
        .unwrap()
        .into_iter()
        .map(|(id, value)| (id, value.expose_secret().to_vec()))
        .collect()
}

async fn meets_the_secret_store_contract(server: Server) {
    let kv = KvServer::start(server).await;
    let store = VaultStore::connect(kv.root_config()).await.unwrap();
    secrets::secret_store(&store, &Legacy(&kv)).await;
}

async fn each_value_is_a_secret_of_its_own_beside_the_credentials_kept_before(server: Server) {
    let kv = KvServer::start(server).await;
    let mut config = kv.root_config();
    config.mount = format!("{MOUNT}/");
    config.path_prefix = format!("/{PREFIX}/");
    let store = VaultStore::connect(config).await.unwrap();
    let adam = user();
    Legacy(&kv).keep(adam, "GH_TOKEN", "ghp_kept").await;
    let (a, b) = (SecretId::generate(), SecretId::generate());
    store.put(adam, a, &bytes("a")).await.unwrap();
    store
        .put(adam, b, &SecretSlice::from(vec![0, 255]))
        .await
        .unwrap();

    for (id, encoded) in [(a, "YQ=="), (b, "AP8=")] {
        let stored: BTreeMap<String, String> =
            kv2::read(&kv.root, MOUNT, &path(adam, id)).await.unwrap();
        assert_eq!(stored, BTreeMap::from([("base64".into(), encoded.into())]));
    }
    let mut listed = kv2::list(&kv.root, MOUNT, &format!("{}/", owner_path(adam)))
        .await
        .unwrap();
    listed.sort();
    let mut ids = vec![a.0.to_string(), b.0.to_string()];
    ids.sort();
    assert_eq!(listed, ids);
    assert_eq!(
        exposed(&store.legacy_credentials(adam).await.unwrap()),
        ["GH_TOKEN=ghp_kept"]
    );
    store.forget_legacy_credentials(adam).await.unwrap();
    assert!(store.legacy_credentials(adam).await.unwrap().is_empty());
    assert_eq!(read(&store, adam, &[a, b]).await.len(), 2);
}

async fn concurrent_writes_for_one_owner_lose_nothing(server: Server) {
    let kv = KvServer::start(server).await;
    let store = Arc::new(VaultStore::connect(kv.root_config()).await.unwrap());
    let adam = user();
    let ids: Vec<SecretId> = (0..8).map(|_| SecretId::generate()).collect();
    let mut writers = JoinSet::new();
    for (i, id) in ids.iter().copied().enumerate() {
        let store = Arc::clone(&store);
        writers.spawn(async move { store.put(adam, id, &bytes(&format!("v{i}"))).await });
    }
    while let Some(written) = writers.join_next().await {
        written.unwrap().unwrap();
    }
    let found = read(&store, adam, &ids).await;
    for (i, id) in ids.iter().enumerate() {
        assert_eq!(found[id], format!("v{i}").into_bytes());
    }
}

async fn an_overwritten_value_leaves_no_readable_history(server: Server) {
    let kv = KvServer::start(server).await;
    let store = VaultStore::connect(kv.root_config()).await.unwrap();
    let adam = user();
    let id = SecretId::generate();
    store.put(adam, id, &bytes("first")).await.unwrap();
    store.put(adam, id, &bytes("second")).await.unwrap();
    assert_eq!(
        readable_versions(&kv, &path(adam, id)).await,
        [BTreeMap::from([("base64".into(), "c2Vjb25k".into())])]
    );
    assert!(store.remove(adam, id).await.unwrap());
    assert!(!store.remove(adam, id).await.unwrap());
    assert_eq!(readable_versions(&kv, &path(adam, id)).await, []);
    assert!(read(&store, adam, &[id]).await.is_empty());
}

// Probes every version number the store could have written, not only those metadata lists.
async fn readable_versions(kv: &KvServer, path: &str) -> Vec<BTreeMap<String, String>> {
    let mut readable = Vec::new();
    for version in 1..=10 {
        if let Ok(data) = kv2::read_version(&kv.root, MOUNT, path, version).await {
            readable.push(data);
        }
    }
    readable
}

async fn a_soft_deleted_value_is_left_out(server: Server) {
    let kv = KvServer::start(server).await;
    let store = VaultStore::connect(kv.root_config()).await.unwrap();
    let adam = user();
    let (gone, kept) = (SecretId::generate(), SecretId::generate());
    store.put(adam, gone, &bytes("a")).await.unwrap();
    store.put(adam, kept, &bytes("b")).await.unwrap();
    kv2::delete_latest(&kv.root, MOUNT, &path(adam, gone))
        .await
        .unwrap();
    assert_eq!(
        read(&store, adam, &[gone, kept]).await,
        BTreeMap::from([(kept, b"b".to_vec())])
    );
    store.put(adam, gone, &bytes("again")).await.unwrap();
    assert_eq!(read(&store, adam, &[gone]).await[&gone], b"again");
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
        kv2::set(&kv.root, MOUNT, &owner_path(adam), &data)
            .await
            .unwrap();
        match store.legacy_credentials(adam).await {
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
    let id = SecretId::generate();
    for data in [
        json!({"value": "sk-quoted-3"}),
        json!({"base64": "sk-quoted-4!"}),
    ] {
        kv2::set(&kv.root, MOUNT, &path(adam, id), &data)
            .await
            .unwrap();
        match store.get(adam, &[id]).await {
            Err(error @ Error::Internal { .. }) => {
                let message = error.to_string();
                assert!(
                    message.contains(&id.0.to_string()) && !message.contains("sk-quoted"),
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
    let (a, b) = (SecretId::generate(), SecretId::generate());
    store.put(adam, a, &bytes("a")).await.unwrap();

    std::fs::write(&jwt_path, &second).unwrap();
    tokio::time::sleep(Duration::from_secs(4)).await;
    store.put(adam, b, &bytes("b")).await.unwrap();

    assert_eq!(
        read(&store, adam, &[a, b]).await,
        BTreeMap::from([(a, b"a".to_vec()), (b, b"b".to_vec())])
    );
    assert_eq!(*reviewed.lock().unwrap(), [first, second]);

    store.remove(adam, a).await.unwrap();
    store.remove(adam, b).await.unwrap();
    assert!(read(&store, adam, &[a, b]).await.is_empty());
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
path "{MOUNT}/metadata/{PREFIX}/*" {{ capabilities = ["read", "create", "update", "delete", "list"] }}"#
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
