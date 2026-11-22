use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use corral_core::Error;
use corral_core::contract::secrets;
use corral_core::domain::{CredentialName, UserId};
use corral_core::ports::SecretStore;
use corral_secrets_database::{DatabaseSecretStore, Keyring};
use corral_sql::testing::{PostgresServer, Sqlite};
use corral_sql::{Pool, on_pool};
use secrecy::{ExposeSecret, SecretString};
use uuid::Uuid;

async fn database_secret_store(pool: &Pool) {
    secrets::secret_store(&sealed(pool, &[(1, 1)]).await).await;
    stored_values_never_contain_the_plaintext(pool).await;
    a_wrong_key_fails_to_decrypt_but_names_still_list(pool).await;
    a_value_moved_to_another_owner_or_name_fails_to_decrypt(pool).await;
    a_rotated_keyring_reads_old_values_and_seals_with_the_new_key(pool).await;
}

#[tokio::test]
async fn sqlite_meets_the_secret_store_contract() {
    let sqlite = Sqlite::new();
    database_secret_store(&sqlite.pool().await).await;
}

#[tokio::test]
async fn postgres_meets_the_secret_store_contract() {
    let postgres = PostgresServer::start().await;
    database_secret_store(&postgres.pool().await).await;
}

async fn stored_values_never_contain_the_plaintext(pool: &Pool) {
    let secrets = sealed(pool, &[(1, 1)]).await;
    let adam = user();
    let value = "sk-live-0123456789abcdef";
    secrets
        .set(adam, &name("A_KEY"), secret(value))
        .await
        .unwrap();
    let first = stored(pool, adam).await;
    secrets
        .set(adam, &name("A_KEY"), secret(value))
        .await
        .unwrap();
    let second = stored(pool, adam).await;

    assert_ne!(first, second);
    for (_, sealed) in first.iter().chain(&second) {
        assert!(!sealed.windows(value.len()).any(|w| w == value.as_bytes()));
    }
    assert_eq!(
        secrets.load(adam).await.unwrap()[&name("A_KEY")].expose_secret(),
        value
    );
}

async fn a_wrong_key_fails_to_decrypt_but_names_still_list(pool: &Pool) {
    let adam = user();
    sealed(pool, &[(1, 1)])
        .await
        .set(adam, &name("A_KEY"), secret("adam-secret"))
        .await
        .unwrap();

    let wrong = sealed(pool, &[(1, 9)]).await;
    assert_eq!(wrong.names(adam).await.unwrap(), [name("A_KEY")]);
    fails_to_decrypt(&wrong, adam, "A_KEY").await;
}

async fn a_value_moved_to_another_owner_or_name_fails_to_decrypt(pool: &Pool) {
    let secrets = sealed(pool, &[(1, 1)]).await;
    let (adam, eve) = (user(), user());
    secrets
        .set(adam, &name("A_KEY"), secret("adam-secret"))
        .await
        .unwrap();

    on_pool!(pool, |pool| {
        sqlx::query(
            "insert into credentials (user_id, name, sealed) \
             select $2, name, sealed from credentials where user_id = $1",
        )
        .bind(adam.0)
        .bind(eve.0)
        .execute(pool)
        .await
        .map(|_| ())
    })
    .unwrap();
    on_pool!(pool, |pool| {
        sqlx::query(
            "insert into credentials (user_id, name, sealed) \
             select user_id, 'B_KEY', sealed from credentials where user_id = $1",
        )
        .bind(adam.0)
        .execute(pool)
        .await
        .map(|_| ())
    })
    .unwrap();

    fails_to_decrypt(&secrets, eve, "A_KEY").await;
    fails_to_decrypt(&secrets, adam, "B_KEY").await;
}

async fn a_rotated_keyring_reads_old_values_and_seals_with_the_new_key(pool: &Pool) {
    let adam = user();
    sealed(pool, &[(1, 1)])
        .await
        .set(adam, &name("OLD"), secret("old"))
        .await
        .unwrap();

    let rotated = sealed(pool, &[(2, 2), (1, 1)]).await;
    rotated
        .set(adam, &name("NEW"), secret("new"))
        .await
        .unwrap();
    let env = rotated.load(adam).await.unwrap();
    assert_eq!(env[&name("OLD")].expose_secret(), "old");
    assert_eq!(env[&name("NEW")].expose_secret(), "new");
    let versions: Vec<_> = stored(pool, adam)
        .await
        .into_iter()
        .map(|(name, sealed)| (name, sealed[0]))
        .collect();
    assert_eq!(versions, [("NEW".to_owned(), 2), ("OLD".to_owned(), 1)]);

    let message = fails_to_decrypt(&sealed(pool, &[(1, 1)]).await, adam, "NEW").await;
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

async fn sealed(pool: &Pool, keys: &[(u8, u8)]) -> DatabaseSecretStore {
    let value = keys
        .iter()
        .map(|(version, byte)| format!("{version}:{}", STANDARD.encode([*byte; 32])))
        .collect::<Vec<_>>()
        .join(",");
    DatabaseSecretStore::open(pool.clone(), Keyring::from_env(&value).unwrap())
        .await
        .expect("open the secret store")
}

async fn stored(pool: &Pool, user: UserId) -> Vec<(String, Vec<u8>)> {
    on_pool!(pool, |pool| {
        sqlx::query_as("select name, sealed from credentials where user_id = $1 order by name")
            .bind(user.0)
            .fetch_all(pool)
            .await
    })
    .unwrap()
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
