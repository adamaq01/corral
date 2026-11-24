use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use corral_core::Error;
use corral_core::contract::secrets::{self, LegacyWriter};
use corral_core::domain::{SecretId, UserId, VariableName};
use corral_core::fakes::exposed;
use corral_core::ports::SecretStore;
use corral_secrets_database::{DatabaseSecretStore, Keyring};
use corral_sql::testing::Databases;
use corral_sql::{Pool, on_pool};
use secrecy::{ExposeSecret, SecretSlice};
use uuid::Uuid;

struct Legacy<'a>(&'a DatabaseSecretStore);

#[async_trait]
impl LegacyWriter for Legacy<'_> {
    async fn keep(&self, owner: UserId, name: &str, value: &str) {
        self.0
            .keep_legacy(owner, &VariableName::parse(name).unwrap(), value)
            .await;
    }
}

async fn database_secret_store(pool: &Pool) {
    let store = sealed(pool, &[(1, 1)]).await;
    secrets::secret_store(&store, &Legacy(&store)).await;
    stored_values_never_contain_the_plaintext(pool).await;
    a_wrong_key_fails_to_decrypt(pool).await;
    a_value_moved_to_another_owner_or_id_fails_to_decrypt(pool).await;
    a_rotated_keyring_reads_old_values_and_seals_with_the_new_key(pool).await;
    credentials_the_previous_release_sealed_read_back(pool).await;
}

#[tokio::test]
async fn sqlite_meets_the_secret_store_contract() {
    let sqlite = Databases::sqlite();
    database_secret_store(&sqlite.pool().await).await;
}

#[tokio::test]
async fn postgres_meets_the_secret_store_contract() {
    let postgres = Databases::postgres().await;
    database_secret_store(&postgres.pool().await).await;
}

async fn stored_values_never_contain_the_plaintext(pool: &Pool) {
    let secrets = sealed(pool, &[(1, 1)]).await;
    let adam = user();
    let id = SecretId::generate();
    let value = "sk-live-0123456789abcdef";
    secrets.put(adam, id, &bytes(value)).await.unwrap();
    let first = stored(pool, adam).await;
    secrets.put(adam, id, &bytes(value)).await.unwrap();
    let second = stored(pool, adam).await;

    assert_ne!(first, second);
    for (_, sealed) in first.iter().chain(&second) {
        assert!(!sealed.windows(value.len()).any(|w| w == value.as_bytes()));
    }
    assert_eq!(read(&secrets, adam, id).await.unwrap(), value.as_bytes());
}

async fn a_wrong_key_fails_to_decrypt(pool: &Pool) {
    let adam = user();
    let id = SecretId::generate();
    sealed(pool, &[(1, 1)])
        .await
        .put(adam, id, &bytes("adam-secret"))
        .await
        .unwrap();
    fails_to_decrypt(&sealed(pool, &[(1, 9)]).await, adam, id).await;
}

async fn a_value_moved_to_another_owner_or_id_fails_to_decrypt(pool: &Pool) {
    let secrets = sealed(pool, &[(1, 1)]).await;
    let (adam, eve) = (user(), user());
    let (id, other) = (SecretId::generate(), SecretId::generate());
    secrets.put(adam, id, &bytes("adam-secret")).await.unwrap();

    on_pool!(pool, |pool| {
        sqlx::query(
            "insert into secrets (user_id, id, sealed) \
             select $2, id, sealed from secrets where user_id = $1",
        )
        .bind(adam.0)
        .bind(eve.0)
        .execute(pool)
        .await
        .map(drop)
    })
    .unwrap();
    on_pool!(pool, |pool| {
        sqlx::query(
            "insert into secrets (user_id, id, sealed) \
             select user_id, $2, sealed from secrets where user_id = $1",
        )
        .bind(adam.0)
        .bind(other.0)
        .execute(pool)
        .await
        .map(drop)
    })
    .unwrap();

    fails_to_decrypt(&secrets, eve, id).await;
    fails_to_decrypt(&secrets, adam, other).await;
}

async fn a_rotated_keyring_reads_old_values_and_seals_with_the_new_key(pool: &Pool) {
    let adam = user();
    let (old, new) = (SecretId::generate(), SecretId::generate());
    sealed(pool, &[(1, 1)])
        .await
        .put(adam, old, &bytes("old"))
        .await
        .unwrap();

    let rotated = sealed(pool, &[(2, 2), (1, 1)]).await;
    rotated.put(adam, new, &bytes("new")).await.unwrap();
    assert_eq!(read(&rotated, adam, old).await.unwrap(), b"old");
    assert_eq!(read(&rotated, adam, new).await.unwrap(), b"new");
    let mut versions: Vec<(Uuid, u8)> = stored(pool, adam)
        .await
        .into_iter()
        .map(|(id, sealed)| (id, sealed[0]))
        .collect();
    versions.sort();
    let mut expected = vec![(old.0, 1), (new.0, 2)];
    expected.sort();
    assert_eq!(versions, expected);

    let message = fails_to_decrypt(&sealed(pool, &[(1, 1)]).await, adam, new).await;
    assert!(message.contains("key version 2"), "{message}");
}

/// Rows the release before entries wrote, for this user with this key.
async fn credentials_the_previous_release_sealed_read_back(pool: &Pool) {
    let adam = UserId(Uuid::parse_str("0b9c6f0e-1f3e-4a4e-9a52-6a1c0f1d2e3f").unwrap());
    for (name, sealed) in [
        (
            "GH_TOKEN",
            "0194525f9a08b3c382feaf5a0b32bd53bca7d8dd513ec77b2500f81dd27589b47a623304d23bdbccf4febe4ac2fce26281c89f069f34e4b845ccd74a72c45fbfc97ba7fe90",
        ),
        (
            "SHORT",
            "0181c5fce52ace042e9d5ae026380f39e70768890bdb48d5a33aa07f1d445ad29967cebf",
        ),
    ] {
        let sealed: Vec<u8> = (0..sealed.len())
            .step_by(2)
            .map(|at| u8::from_str_radix(&sealed[at..at + 2], 16).unwrap())
            .collect();
        on_pool!(pool, |pool| {
            sqlx::query("insert into credentials (user_id, name, sealed) values ($1, $2, $3)")
                .bind(adam.0)
                .bind(name)
                .bind(&sealed)
                .execute(pool)
                .await
                .map(drop)
        })
        .unwrap();
    }
    let store = keyed(pool, &format!("1:{}", STANDARD.encode([7; 32]))).await;
    assert_eq!(
        exposed(&store.legacy_credentials(adam).await.unwrap()),
        [
            "GH_TOKEN=ghp_0123456789abcdefghijklmnopqrstuvwxyz",
            "SHORT=hunter2"
        ]
    );
    store.forget_legacy_credentials(adam).await.unwrap();
    assert!(store.legacy_credentials(adam).await.unwrap().is_empty());
}

async fn read(secrets: &DatabaseSecretStore, owner: UserId, id: SecretId) -> Option<Vec<u8>> {
    secrets
        .get(owner, &[id])
        .await
        .unwrap()
        .remove(&id)
        .map(|value| value.expose_secret().to_vec())
}

async fn fails_to_decrypt(secrets: &DatabaseSecretStore, owner: UserId, id: SecretId) -> String {
    match secrets.get(owner, &[id]).await {
        Err(error @ Error::Internal { .. }) => {
            let message = format!("{error} {error:?}");
            assert!(message.contains(&id.0.to_string()), "{message}");
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
    keyed(pool, &value).await
}

async fn keyed(pool: &Pool, keys: &str) -> DatabaseSecretStore {
    DatabaseSecretStore::open(pool.clone(), Keyring::from_env(keys).unwrap())
        .await
        .expect("open the secret store")
}

async fn stored(pool: &Pool, owner: UserId) -> Vec<(Uuid, Vec<u8>)> {
    on_pool!(pool, |pool| {
        sqlx::query_as("select id, sealed from secrets where user_id = $1 order by id")
            .bind(owner.0)
            .fetch_all(pool)
            .await
    })
    .unwrap()
}

fn user() -> UserId {
    UserId(Uuid::new_v4())
}

fn bytes(value: &str) -> SecretSlice<u8> {
    SecretSlice::from(value.as_bytes().to_vec())
}
