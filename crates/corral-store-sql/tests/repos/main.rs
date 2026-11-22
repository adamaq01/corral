mod contract;
mod secrets;

use std::sync::Arc;

use chrono::{DateTime, TimeDelta, Utc};
use corral_core::Error;
use corral_core::domain::{
    Handle, Role, Sandbox, SandboxId, SandboxName, SshKey, SshPublicKey, User, UserId,
};
use corral_core::ports::{KeyRepo, SandboxRepo, UserRepo};
use corral_store_sql::Store;
use sqlx::{Connection, SqliteConnection};
use tempfile::TempDir;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::ImageExt;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use uuid::Uuid;

async fn meets_every_repo_contract(url: &str) {
    Store::connect(url).await.expect("connect");
    let store = Arc::new(
        Store::connect(url)
            .await
            .expect("reconnect to a migrated database"),
    );
    contract::user_repo(&*store).await;
    contract::key_repo(&*store, &*store).await;
    contract::sandbox_repo(&*store, &*store).await;
    contract::local_account_repo(&*store).await;
    secrets::database_secret_store(&store, url).await;
}

fn sqlite_url(dir: &TempDir) -> String {
    format!("sqlite://{}", dir.path().join("corral.db").display())
}

#[tokio::test]
async fn sqlite_meets_every_repo_contract() {
    let dir = tempfile::tempdir().expect("temp dir");
    meets_every_repo_contract(&sqlite_url(&dir)).await;
}

#[tokio::test]
async fn postgres_meets_every_repo_contract() {
    let postgres = Postgres::default()
        .with_tag("19")
        .start()
        .await
        .expect("start postgres");
    let url = format!(
        "postgres://postgres:postgres@{}:{}/postgres",
        postgres.get_host().await.unwrap(),
        postgres.get_host_port_ipv4(5432).await.unwrap()
    );
    meets_every_repo_contract(&url).await;
}

#[tokio::test]
async fn connect_names_only_the_scheme_it_refuses() {
    for unsupported in ["mysql://u:secret-pw@db/corral", "corral.db"] {
        match Store::connect(unsupported).await {
            Err(Error::Invalid(message)) => {
                assert!(message.contains("sqlite: or postgres:"), "{message}");
                assert!(!message.contains("secret-pw"), "{message}");
            }
            Err(other) => panic!("{unsupported}: {other:?}"),
            Ok(_) => panic!("{unsupported}: connected"),
        }
    }
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
async fn sqlite_connect_creates_a_wal_database_that_enforces_foreign_keys() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = Store::connect(&sqlite_url(&dir)).await.expect("connect");
    assert!(dir.path().join("corral.db").exists());

    let mut raw = SqliteConnection::connect(&sqlite_url(&dir)).await.unwrap();
    let mode: String = sqlx::query_scalar("pragma journal_mode")
        .fetch_one(&mut raw)
        .await
        .unwrap();
    assert_eq!(mode, "wal");

    let orphan = SshKey {
        id: Uuid::new_v4(),
        user_id: UserId(Uuid::new_v4()),
        label: "laptop".to_owned(),
        public_key: SshPublicKey::parse(include_str!(
            "../../../corral-core/tests/fixtures/adam_ed25519.pub"
        ))
        .unwrap(),
        created_at: Utc::now(),
    };
    assert!(matches!(
        KeyRepo::insert(&store, &orphan).await,
        Err(Error::Internal { .. })
    ));
}

#[tokio::test]
async fn sqlite_sub_second_timestamps_round_trip_in_order() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = Store::connect(&sqlite_url(&dir)).await.expect("connect");
    let owner = user();
    UserRepo::insert(&store, &owner).await.unwrap();
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

    for sandbox in chronological.iter().rev() {
        SandboxRepo::insert(&store, sandbox).await.unwrap();
    }
    assert_eq!(
        SandboxRepo::list(&store, owner.id).await.unwrap(),
        chronological
    );
}
