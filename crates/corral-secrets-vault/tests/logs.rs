mod support;

use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex};

use corral_core::domain::{CredentialName, UserId};
use corral_core::ports::SecretStore;
use corral_secrets_vault::VaultStore;
use secrecy::SecretString;
use support::{KvServer, MOUNT, PREFIX, Server};
use tracing_subscriber::filter::LevelFilter;
use uuid::Uuid;
use vaultrs::kv2;

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
    tracing_subscriber::fmt()
        .with_max_level(LevelFilter::TRACE)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .init();

    for server in [Server::OpenBao, Server::Vault] {
        let kv = KvServer::start(server).await;
        let store = VaultStore::connect(kv.root_config()).await.unwrap();
        let (adam, key) = (
            UserId(Uuid::new_v4()),
            CredentialName::parse("CANARY_KEY").unwrap(),
        );
        let value = "sk-canary-0123456789";
        store
            .set(adam, &key, SecretString::from(value))
            .await
            .unwrap();
        store.load(adam).await.unwrap();
        let stored: BTreeMap<String, String> =
            kv2::read(&kv.root, MOUNT, &format!("{PREFIX}/{}", adam.0))
                .await
                .unwrap();
        assert_eq!(stored["CANARY_KEY"], value);
        store.remove(adam, &key).await.unwrap();

        let logs = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
        assert!(
            logs.contains("Sending Request") && logs.contains(&adam.0.to_string()),
            "the capture missed the store's requests"
        );
        assert!(!logs.contains(value));
    }
}
