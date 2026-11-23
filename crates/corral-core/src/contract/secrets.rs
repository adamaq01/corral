use std::collections::BTreeMap;

use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretSlice};
use uuid::Uuid;

use crate::domain::{SecretId, UserId};
use crate::fakes::{MemorySecretStore, credential, exposed};
use crate::ports::SecretStore;

/// Keeps a credential the way the store under test kept them before entries existed.
#[async_trait]
pub trait LegacyWriter: Send + Sync {
    async fn keep(&self, owner: UserId, name: &str, value: &str);
}

#[async_trait]
impl LegacyWriter for MemorySecretStore {
    async fn keep(&self, owner: UserId, name: &str, value: &str) {
        self.keep_legacy(owner, &credential(name), value);
    }
}

fn user() -> UserId {
    UserId(Uuid::new_v4())
}

fn value(bytes: &[u8]) -> SecretSlice<u8> {
    SecretSlice::from(bytes.to_vec())
}

pub async fn secret_store(store: &dyn SecretStore, legacy: &dyn LegacyWriter) {
    values_round_trip_as_bytes(store).await;
    get_leaves_out_ids_without_a_value(store).await;
    remove_reports_whether_there_was_a_value(store).await;
    owners_are_isolated(store).await;
    legacy_credentials_read_back_until_forgotten(store, legacy).await;
    remove_all_clears_only_that_owner(store, legacy).await;
}

async fn read(
    store: &dyn SecretStore,
    owner: UserId,
    ids: &[SecretId],
) -> BTreeMap<SecretId, Vec<u8>> {
    store
        .get(owner, ids)
        .await
        .expect("get")
        .into_iter()
        .map(|(id, value)| (id, value.expose_secret().to_vec()))
        .collect()
}

async fn values_round_trip_as_bytes(store: &dyn SecretStore) {
    let adam = user();
    let (text, binary, empty, large) = (
        SecretId::generate(),
        SecretId::generate(),
        SecretId::generate(),
        SecretId::generate(),
    );
    let big: Vec<u8> = (0..=255).cycle().take(256 << 10).collect();
    for (id, bytes) in [
        (text, b"first".as_slice()),
        (binary, &[0, 159, 146, 150, 255]),
        (empty, b""),
        (large, &big),
        (text, b"ghp_\xc3\xa9 \"quoted\" \\ second"),
    ] {
        store.put(adam, id, &value(bytes)).await.expect("put");
    }
    let found = read(store, adam, &[text, binary, empty, large]).await;
    assert_eq!(
        found,
        BTreeMap::from([
            (text, b"ghp_\xc3\xa9 \"quoted\" \\ second".to_vec()),
            (binary, vec![0, 159, 146, 150, 255]),
            (empty, Vec::new()),
            (large, big),
        ])
    );
    let debug = format!("{:?}", store.get(adam, &[text]).await.unwrap());
    assert!(!debug.contains("ghp_"), "{debug}");
}

async fn get_leaves_out_ids_without_a_value(store: &dyn SecretStore) {
    let adam = user();
    let kept = SecretId::generate();
    store.put(adam, kept, &value(b"kept")).await.expect("put");
    let missing = SecretId::generate();
    assert_eq!(
        read(store, adam, &[missing, kept]).await,
        BTreeMap::from([(kept, b"kept".to_vec())])
    );
    assert!(read(store, adam, &[]).await.is_empty());
    assert!(read(store, user(), &[kept]).await.is_empty());
}

async fn remove_reports_whether_there_was_a_value(store: &dyn SecretStore) {
    let adam = user();
    let (a, b) = (SecretId::generate(), SecretId::generate());
    for id in [a, b] {
        store.put(adam, id, &value(b"x")).await.expect("put");
    }
    assert!(store.remove(adam, b).await.expect("remove"));
    assert!(!store.remove(adam, b).await.expect("remove again"));
    assert_eq!(
        read(store, adam, &[a, b]).await,
        BTreeMap::from([(a, b"x".to_vec())])
    );
    assert!(!store.remove(user(), a).await.unwrap());
    assert_eq!(read(store, adam, &[a]).await.len(), 1);
}

async fn owners_are_isolated(store: &dyn SecretStore) {
    let (adam, eve) = (user(), user());
    let id = SecretId::generate();
    store.put(adam, id, &value(b"adams")).await.expect("put");
    store.put(eve, id, &value(b"eves")).await.expect("put");
    assert_eq!(read(store, adam, &[id]).await[&id], b"adams");
    assert_eq!(read(store, eve, &[id]).await[&id], b"eves");
    assert!(store.remove(eve, id).await.unwrap());
    assert_eq!(read(store, adam, &[id]).await[&id], b"adams");
}

async fn legacy_credentials_read_back_until_forgotten(
    store: &dyn SecretStore,
    legacy: &dyn LegacyWriter,
) {
    let (adam, eve) = (user(), user());
    legacy.keep(adam, "GH_TOKEN", "ghp_adams").await;
    legacy.keep(adam, "A_KEY", "a \"quoted\" value").await;
    legacy.keep(eve, "GH_TOKEN", "ghp_eves").await;
    let kept = SecretId::generate();
    store.put(adam, kept, &value(b"entry")).await.expect("put");

    assert_eq!(
        exposed(&store.legacy_credentials(adam).await.unwrap()),
        ["A_KEY=a \"quoted\" value", "GH_TOKEN=ghp_adams"]
    );
    assert!(store.legacy_credentials(user()).await.unwrap().is_empty());
    store.forget_legacy_credentials(adam).await.expect("forget");
    store
        .forget_legacy_credentials(adam)
        .await
        .expect("forget again");
    store
        .forget_legacy_credentials(user())
        .await
        .expect("forget a stranger's");
    assert!(store.legacy_credentials(adam).await.unwrap().is_empty());
    assert_eq!(read(store, adam, &[kept]).await[&kept], b"entry");
    assert_eq!(
        exposed(&store.legacy_credentials(eve).await.unwrap()),
        ["GH_TOKEN=ghp_eves"]
    );
}

async fn remove_all_clears_only_that_owner(store: &dyn SecretStore, legacy: &dyn LegacyWriter) {
    let (adam, eve) = (user(), user());
    let ids = [SecretId::generate(), SecretId::generate()];
    for id in ids {
        store.put(adam, id, &value(b"adams")).await.expect("put");
        store.put(eve, id, &value(b"eves")).await.expect("put");
    }
    legacy.keep(adam, "OLD", "old").await;
    legacy.keep(eve, "OLD", "old").await;

    store.remove_all(adam).await.expect("remove all");
    assert!(read(store, adam, &ids).await.is_empty());
    assert!(store.legacy_credentials(adam).await.unwrap().is_empty());
    assert_eq!(read(store, eve, &ids).await.len(), 2);
    assert_eq!(
        exposed(&store.legacy_credentials(eve).await.unwrap()),
        ["OLD=old"]
    );
    store.remove_all(adam).await.expect("remove all again");
    store
        .remove_all(user())
        .await
        .expect("remove all of a stranger");

    let fresh = SecretId::generate();
    store.put(adam, fresh, &value(b"fresh")).await.expect("put");
    assert_eq!(read(store, adam, &[fresh]).await[&fresh], b"fresh");
}
