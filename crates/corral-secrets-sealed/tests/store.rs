use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use corral_core::Error;
use corral_core::contract::secrets;
use corral_core::domain::{CredentialName, UserId};
use corral_core::fakes::{MemorySecretBlobRepo, MemoryUserRepo};
use corral_core::ports::{SecretBlobRepo, SecretStore};
use corral_secrets_sealed::{Keyring, SealedStore};
use secrecy::{ExposeSecret, SecretString};
use uuid::Uuid;

fn keys(entries: &[(u8, u8)]) -> Keyring {
    let value = entries
        .iter()
        .map(|(version, byte)| format!("{version}:{}", STANDARD.encode([*byte; 32])))
        .collect::<Vec<_>>()
        .join(",");
    Keyring::from_env(&value).unwrap()
}

fn store(blobs: &Arc<MemorySecretBlobRepo>, keyring: Keyring) -> SealedStore {
    SealedStore::new(blobs.clone(), keyring)
}

fn user() -> UserId {
    UserId(Uuid::new_v4())
}

fn name(value: &str) -> CredentialName {
    CredentialName::parse(value).unwrap()
}

fn secret(value: &str) -> SecretString {
    SecretString::from(value)
}

#[tokio::test]
async fn meets_the_secret_store_contract() {
    let blobs = Arc::new(MemorySecretBlobRepo::default());
    secrets::secret_store(&MemoryUserRepo::default(), &store(&blobs, keys(&[(1, 1)]))).await;
}

#[tokio::test]
async fn stored_blobs_never_contain_the_plaintext() {
    let blobs = Arc::new(MemorySecretBlobRepo::default());
    let store = store(&blobs, keys(&[(1, 1)]));
    let (adam, key) = (user(), name("OPENAI_API_KEY"));
    let value = "sk-live-0123456789abcdef";

    store.put(adam, &key, secret(value)).await.unwrap();
    let first = blobs.load_all(adam).await.unwrap();
    store.put(adam, &key, secret(value)).await.unwrap();
    let second = blobs.load_all(adam).await.unwrap();

    assert_ne!(first, second);
    for (_, blob) in first.iter().chain(&second) {
        assert!(!blob.windows(value.len()).any(|w| w == value.as_bytes()));
    }
    assert_eq!(
        store.load_all(adam).await.unwrap()[&key].expose_secret(),
        value
    );
}

#[tokio::test]
async fn a_blob_moved_to_another_owner_or_name_does_not_open() {
    let blobs = Arc::new(MemorySecretBlobRepo::default());
    let store = store(&blobs, keys(&[(1, 1)]));
    let (adam, eve) = (user(), user());
    store
        .put(adam, &name("A_KEY"), secret("adam-secret"))
        .await
        .unwrap();
    let blob = blobs.load_all(adam).await.unwrap().remove(0).1;

    blobs.put(eve, &name("A_KEY"), blob.clone()).await.unwrap();
    blobs.put(adam, &name("B_KEY"), blob).await.unwrap();

    for owner in [adam, eve] {
        match store.load_all(owner).await {
            Err(Error::Secrets(message)) => assert!(!message.contains("adam-secret"), "{message}"),
            other => panic!("expected a secrets error, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn a_rotated_keyring_opens_old_values_and_seals_with_the_new_key() {
    let blobs = Arc::new(MemorySecretBlobRepo::default());
    let adam = user();
    store(&blobs, keys(&[(1, 1)]))
        .put(adam, &name("OLD"), secret("old"))
        .await
        .unwrap();

    let rotated = store(&blobs, keys(&[(2, 2), (1, 1)]));
    rotated
        .put(adam, &name("NEW"), secret("new"))
        .await
        .unwrap();
    let env = rotated.load_all(adam).await.unwrap();
    assert_eq!(env[&name("OLD")].expose_secret(), "old");
    assert_eq!(env[&name("NEW")].expose_secret(), "new");

    let mut versions: Vec<_> = blobs
        .load_all(adam)
        .await
        .unwrap()
        .into_iter()
        .map(|(name, blob)| (name.to_string(), blob[0]))
        .collect();
    versions.sort();
    assert_eq!(versions, [("NEW".to_owned(), 2), ("OLD".to_owned(), 1)]);

    match store(&blobs, keys(&[(1, 1)])).load_all(adam).await {
        Err(Error::Secrets(message)) => {
            assert!(
                message.contains("NEW") && message.contains("key version 2"),
                "{message}"
            );
        }
        other => panic!("expected a secrets error, got {other:?}"),
    }
}
