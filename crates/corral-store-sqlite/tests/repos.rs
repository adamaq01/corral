use chrono::{DateTime, TimeDelta, Utc};
use corral_core::Error;
use corral_core::contract::repos;
use corral_core::domain::{Handle, Role, Sandbox, SandboxId, SandboxName, SshKey, User, UserId};
use corral_store_sqlite::SqliteStore;
use sqlx::{Connection, SqliteConnection};
use tempfile::TempDir;
use uuid::Uuid;

fn url(dir: &TempDir) -> String {
    format!("sqlite://{}", dir.path().join("corral.db").display())
}

async fn store() -> (TempDir, SqliteStore) {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = SqliteStore::connect(&url(&dir)).await.expect("connect");
    (dir, store)
}

fn user() -> User {
    User {
        id: UserId(Uuid::new_v4()),
        subject: "subject-adam".to_owned(),
        handle: Handle::parse("adam").unwrap(),
        email: None,
        role: Role::Member,
        enabled: true,
        created_at: Utc::now(),
    }
}

#[tokio::test]
async fn users_meet_the_contract() {
    let (_dir, store) = store().await;
    repos::user_repo(&*store.users()).await;
}

#[tokio::test]
async fn keys_meet_the_contract() {
    let (_dir, store) = store().await;
    repos::key_repo(&*store.users(), &*store.keys()).await;
}

#[tokio::test]
async fn sandboxes_meet_the_contract() {
    let (_dir, store) = store().await;
    repos::sandbox_repo(&*store.users(), &*store.sandboxes()).await;
}

#[tokio::test]
async fn credentials_meet_the_contract() {
    let (_dir, store) = store().await;
    repos::credential_repo(&*store.users(), &*store.credentials()).await;
}

#[tokio::test]
async fn secret_blobs_meet_the_contract() {
    let (_dir, store) = store().await;
    repos::secret_blob_repo(&*store.users(), &*store.secret_blobs()).await;
}

#[tokio::test]
async fn connect_creates_a_wal_database_that_enforces_foreign_keys() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = SqliteStore::connect(&url(&dir)).await.expect("connect");
    assert!(dir.path().join("corral.db").exists());

    let mut raw = SqliteConnection::connect(&url(&dir)).await.unwrap();
    let mode: String = sqlx::query_scalar("pragma journal_mode")
        .fetch_one(&mut raw)
        .await
        .unwrap();
    assert_eq!(mode, "wal");

    let orphan = SshKey {
        id: Uuid::new_v4(),
        user_id: UserId(Uuid::new_v4()),
        label: "laptop".to_owned(),
        public_key: "ssh-ed25519 AAAAorphan".to_owned(),
        fingerprint: "SHA256:orphan".to_owned(),
        created_at: Utc::now(),
    };
    assert!(matches!(
        store.keys().insert(&orphan).await,
        Err(Error::Store(_))
    ));

    SqliteStore::connect(&url(&dir))
        .await
        .expect("reconnect to a migrated database");
}

#[tokio::test]
async fn sub_second_timestamps_round_trip_in_order() {
    let (_dir, store) = store().await;
    let owner = user();
    store.users().insert(&owner).await.unwrap();
    let second = DateTime::from_timestamp(1_800_000_000, 0).unwrap();
    let chronological: Vec<Sandbox> = [
        TimeDelta::zero(),
        TimeDelta::nanoseconds(1),
        TimeDelta::microseconds(1),
        TimeDelta::milliseconds(500),
        TimeDelta::seconds(1),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, offset)| Sandbox {
        id: SandboxId(format!("sbx-{}", 9 - i)),
        owner: owner.id,
        name: SandboxName::parse(&format!("box-{}", 9 - i)).unwrap(),
        image: "default".to_owned(),
        size: "small".to_owned(),
        is_default: i == 0,
        created_at: second + offset,
    })
    .collect();

    let sandboxes = store.sandboxes();
    for sandbox in chronological.iter().rev() {
        sandboxes.insert(sandbox).await.unwrap();
    }
    assert_eq!(sandboxes.list(owner.id).await.unwrap(), chronological);
}
