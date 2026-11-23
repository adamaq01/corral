mod support;

use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use corral_core::Error;
use corral_core::domain::{SecretId, UserId};
use corral_core::ports::SecretStore;
use corral_secrets_vault::{NOISY_LOG_TARGETS, VaultAuth, VaultStore};
use secrecy::{ExposeSecret, SecretSlice, SecretString};
use support::{KvServer, MOUNT, PREFIX, Server};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::reload;
use tracing_subscriber::util::SubscriberInitExt;
use uuid::Uuid;
use vaultrs::api::token::requests::CreateTokenRequest;
use vaultrs::{kv2, token};

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl io::Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn credential_values_never_reach_the_logs() {
    let capture = Capture::default();
    let writer = capture.clone();
    let _scope = tracing_subscriber::fmt()
        .with_max_level(LevelFilter::TRACE)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish()
        .set_default();

    for server in [Server::OpenBao, Server::Vault] {
        let kv = KvServer::start(server).await;
        let store = VaultStore::connect(kv.root_config()).await.unwrap();
        let (adam, id) = (UserId(Uuid::new_v4()), SecretId::generate());
        let value = "sk-canary-0123456789";
        store
            .put(adam, id, &SecretSlice::from(value.as_bytes().to_vec()))
            .await
            .unwrap();
        let read = store.get(adam, &[id]).await.unwrap();
        assert_eq!(read[&id].expose_secret(), value.as_bytes());
        let stored: BTreeMap<String, String> =
            kv2::read(&kv.root, MOUNT, &format!("{PREFIX}/{}/{}", adam.0, id.0))
                .await
                .unwrap();
        assert_eq!(stored["base64"], STANDARD.encode(value));
        store.remove(adam, id).await.unwrap();

        let logs = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
        assert!(
            logs.contains("Sending Request") && logs.contains(&adam.0.to_string()),
            "the capture missed the store's requests"
        );
        assert!(!logs.contains(value) && !logs.contains(&STANDARD.encode(value)));
    }
}

#[tokio::test]
async fn quieted_a_missing_secret_logs_no_error_and_a_real_failure_still_reaches_the_caller() {
    let quiet = NOISY_LOG_TARGETS
        .map(|target| format!(",{target}=off"))
        .concat();
    let quiet = format!("info{quiet}");
    let capture = Capture::default();
    let writer = capture.clone();
    let (filter, reload) = reload::Layer::new(EnvFilter::new("info"));
    let _scope = tracing_subscriber::registry()
        .with(filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(move || writer.clone()),
        )
        .set_default();
    let listen = |directives: &str| {
        reload.reload(EnvFilter::new(directives)).unwrap();
        capture.0.lock().unwrap().clear();
    };
    let logs = || String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();

    for server in [Server::OpenBao, Server::Vault] {
        let kv = KvServer::start(server).await;
        let store = VaultStore::connect(kv.root_config()).await.unwrap();
        let missing = UserId(Uuid::new_v4());
        let nothing = [SecretId::generate()];

        listen("info");
        assert!(store.get(missing, &nothing).await.unwrap().is_empty());
        assert!(
            logs().contains(
                "vaultrs::api: error=The Vault server returned an error (status code 404)"
            ),
            "{server:?}: {}",
            logs()
        );

        listen(&quiet);
        assert!(store.get(missing, &nothing).await.unwrap().is_empty());
        assert_eq!(logs(), "", "{server:?}");

        let unprivileged = token::new(
            &kv.root,
            Some(CreateTokenRequest::builder().policies(vec!["default".to_owned()])),
        )
        .await
        .unwrap()
        .client_token;
        let denied = VaultStore::connect(kv.config(VaultAuth::Token {
            token: SecretString::from(unprivileged),
        }))
        .await
        .unwrap();
        listen(&quiet);
        let loaded = denied.get(missing, &nothing).await;
        assert!(
            matches!(&loaded, Err(Error::Internal { source, .. }) if source.to_string().contains("403")),
            "{server:?}: {loaded:?}"
        );
        assert_eq!(logs(), "", "{server:?}");
    }
}
