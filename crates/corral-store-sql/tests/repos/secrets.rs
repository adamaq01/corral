use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use corral_core::Error;
use corral_core::contract::secrets;
use corral_core::domain::{CredentialName, User, UserId};
use corral_core::ports::SecretStore;
use corral_store_sql::{DatabaseSecretStore, Keyring, Store};
use secrecy::{ExposeSecret, SecretString};
use sqlx::{AnyConnection, Connection};

use crate::contract::seed_user;

pub async fn database_secret_store(store: &Arc<Store>, url: &str) {
    secrets::secret_store(store.as_ref(), &sealed(store, &[(1, 1)])).await;
    stored_values_never_contain_the_plaintext(store, url).await;
    a_wrong_key_fails_to_decrypt_but_names_still_list(store).await;
    a_value_moved_to_another_owner_or_name_fails_to_decrypt(store, url).await;
    a_rotated_keyring_reads_old_values_and_seals_with_the_new_key(store, url).await;
}

async fn stored_values_never_contain_the_plaintext(store: &Arc<Store>, url: &str) {
    let secrets = sealed(store, &[(1, 1)]);
    let adam = seed_user(store.as_ref(), "adam").await;
    let value = "sk-live-0123456789abcdef";
    secrets
        .set(adam.id, &name("A_KEY"), secret(value))
        .await
        .unwrap();
    let first = stored(url, &adam).await;
    secrets
        .set(adam.id, &name("A_KEY"), secret(value))
        .await
        .unwrap();
    let second = stored(url, &adam).await;

    assert_ne!(first, second);
    for (_, sealed) in first.iter().chain(&second) {
        assert!(!sealed.windows(value.len()).any(|w| w == value.as_bytes()));
    }
    assert_eq!(
        secrets.load(adam.id).await.unwrap()[&name("A_KEY")].expose_secret(),
        value
    );
}

async fn a_wrong_key_fails_to_decrypt_but_names_still_list(store: &Arc<Store>) {
    let adam = seed_user(store.as_ref(), "adam").await;
    sealed(store, &[(1, 1)])
        .set(adam.id, &name("A_KEY"), secret("adam-secret"))
        .await
        .unwrap();

    let wrong = sealed(store, &[(1, 9)]);
    assert_eq!(wrong.names(adam.id).await.unwrap(), [name("A_KEY")]);
    fails_to_decrypt(&wrong, adam.id, "A_KEY").await;
}

async fn a_value_moved_to_another_owner_or_name_fails_to_decrypt(store: &Arc<Store>, url: &str) {
    let secrets = sealed(store, &[(1, 1)]);
    let adam = seed_user(store.as_ref(), "adam").await;
    let eve = seed_user(store.as_ref(), "eve").await;
    secrets
        .set(adam.id, &name("A_KEY"), secret("adam-secret"))
        .await
        .unwrap();

    execute(
        url,
        "insert into credentials (user_id, name, sealed) \
         select (select id from users where handle = $2), name, sealed from credentials \
         where user_id = (select id from users where handle = $1)",
        &[adam.handle.as_str(), eve.handle.as_str()],
    )
    .await;
    execute(
        url,
        "insert into credentials (user_id, name, sealed) \
         select user_id, 'B_KEY', sealed from credentials \
         where user_id = (select id from users where handle = $1)",
        &[adam.handle.as_str()],
    )
    .await;

    fails_to_decrypt(&secrets, eve.id, "A_KEY").await;
    fails_to_decrypt(&secrets, adam.id, "B_KEY").await;
}

async fn a_rotated_keyring_reads_old_values_and_seals_with_the_new_key(
    store: &Arc<Store>,
    url: &str,
) {
    let adam = seed_user(store.as_ref(), "adam").await;
    sealed(store, &[(1, 1)])
        .set(adam.id, &name("OLD"), secret("old"))
        .await
        .unwrap();

    let rotated = sealed(store, &[(2, 2), (1, 1)]);
    rotated
        .set(adam.id, &name("NEW"), secret("new"))
        .await
        .unwrap();
    let env = rotated.load(adam.id).await.unwrap();
    assert_eq!(env[&name("OLD")].expose_secret(), "old");
    assert_eq!(env[&name("NEW")].expose_secret(), "new");
    let versions: Vec<_> = stored(url, &adam)
        .await
        .into_iter()
        .map(|(name, sealed)| (name, sealed[0]))
        .collect();
    assert_eq!(versions, [("NEW".to_owned(), 2), ("OLD".to_owned(), 1)]);

    let message = fails_to_decrypt(&sealed(store, &[(1, 1)]), adam.id, "NEW").await;
    assert!(message.contains("key version 2"), "{message}");
}

async fn fails_to_decrypt(secrets: &DatabaseSecretStore, user: UserId, culprit: &str) -> String {
    match secrets.load(user).await {
        Err(error @ Error::Internal { .. }) => {
            let message = format!("{error} {error:?}");
            assert!(message.contains(culprit), "{message}");
            assert!(!message.contains("adam-secret"), "{message}");
            message
        }
        other => panic!("expected a decryption failure, got {other:?}"),
    }
}

fn sealed(store: &Arc<Store>, keys: &[(u8, u8)]) -> DatabaseSecretStore {
    let value = keys
        .iter()
        .map(|(version, byte)| format!("{version}:{}", STANDARD.encode([*byte; 32])))
        .collect::<Vec<_>>()
        .join(",");
    DatabaseSecretStore::new(store.clone(), Keyring::from_env(&value).unwrap())
}

async fn connect(url: &str) -> AnyConnection {
    sqlx::any::install_default_drivers();
    AnyConnection::connect(url).await.expect("raw connection")
}

async fn execute(url: &str, sql: &'static str, binds: &[&str]) {
    let mut query = sqlx::query(sql);
    for bind in binds {
        query = query.bind(*bind);
    }
    query.execute(&mut connect(url).await).await.unwrap();
}

async fn stored(url: &str, user: &User) -> Vec<(String, Vec<u8>)> {
    sqlx::query_as(
        "select name, sealed from credentials \
         where user_id = (select id from users where handle = $1) order by name",
    )
    .bind(user.handle.as_str())
    .fetch_all(&mut connect(url).await)
    .await
    .unwrap()
}

fn name(value: &str) -> CredentialName {
    CredentialName::parse(value).unwrap()
}

fn secret(value: &str) -> SecretString {
    SecretString::from(value)
}
