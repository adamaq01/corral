//! A database the release before the environment wrote, with its users, sandboxes and sealed
//! credentials, upgraded: every credential is a secret variable attached to each sandbox of its
//! owner, with its value.

use std::borrow::Cow;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use corral_core::app::{Environment, Limits, Sandboxes};
use corral_core::domain::{Sandbox, User, UserId};
use corral_core::fakes::{FakeEngine, FakeRegistry, catalog, exposed, links};
use corral_core::ports::{SandboxEngine, SandboxRepo, SandboxSpec, SecretStore, UserRepo};
use corral_secrets_database::{DatabaseSecretStore, Keyring};
use corral_sql::testing::Databases;
use corral_sql::{Migrations, Pool, on_pool};
use corral_store_sql::Store;
use sqlx::migrate::Migrator;
use uuid::Uuid;

const ADAM: &str = "0b9c6f0e-1f3e-4a4e-9a52-6a1c0f1d2e3f";

/// Sealed by the previous release for ADAM with the key `1:` and 32 bytes of 7.
const SEALED: [(&str, &str); 2] = [
    (
        "GH_TOKEN",
        "0194525f9a08b3c382feaf5a0b32bd53bca7d8dd513ec77b2500f81dd27589b47a623304d23bdbccf4febe4ac2fce26281c89f069f34e4b845ccd74a72c45fbfc97ba7fe90",
    ),
    (
        "SHORT",
        "0181c5fce52ace042e9d5ae026380f39e70768890bdb48d5a33aa07f1d445ad29967cebf",
    ),
];

fn first(mut migrator: Migrator, count: usize) -> Migrator {
    migrator.migrations = Cow::Owned(migrator.migrations[..count].to_vec());
    migrator
}

fn hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&text[at..at + 2], 16).unwrap())
        .collect()
}

/// The schema of the release before the environment, with its rows.
async fn previous_release(pool: &Pool, adam: Uuid, eve: Uuid) {
    pool.migrate(Migrations::new(
        "store",
        first(sqlx::migrate!("../corral-store-sql/migrations/sqlite"), 5),
        first(sqlx::migrate!("../corral-store-sql/migrations/postgres"), 5),
    ))
    .await
    .unwrap();
    pool.migrate(Migrations::new(
        "credentials",
        first(
            sqlx::migrate!("../corral-secrets-database/migrations/sqlite"),
            1,
        ),
        first(
            sqlx::migrate!("../corral-secrets-database/migrations/postgres"),
            1,
        ),
    ))
    .await
    .unwrap();
    for (id, handle) in [(adam, "adam"), (eve, "eve")] {
        on_pool!(pool, |pool| {
            sqlx::query(
                "insert into users (id, subject, handle, role, status, created_at) \
                 values ($1, $2, $3, 'member', 'active', $4)",
            )
            .bind(id)
            .bind(format!("local:{handle}"))
            .bind(handle)
            .bind(chrono::Utc::now())
            .execute(pool)
            .await
            .map(drop)
        })
        .unwrap();
    }
    for (owner, name) in [(adam, "main"), (adam, "dev"), (eve, "main")] {
        on_pool!(pool, |pool| {
            sqlx::query(
                "insert into sandboxes (id, owner, name, image, size, created_at) \
                 values ($1, $2, $3, 'base', 'small', $4)",
            )
            .bind(format!(
                "sbx-{}",
                &Uuid::new_v4().simple().to_string()[..12]
            ))
            .bind(owner)
            .bind(name)
            .bind(chrono::Utc::now())
            .execute(pool)
            .await
            .map(drop)
        })
        .unwrap();
    }
    for (name, sealed) in SEALED {
        on_pool!(pool, |pool| {
            sqlx::query("insert into credentials (user_id, name, sealed) values ($1, $2, $3)")
                .bind(adam)
                .bind(name)
                .bind(hex(sealed))
                .execute(pool)
                .await
                .map(drop)
        })
        .unwrap();
    }
}

async fn upgrades_keeping_every_credential(databases: Databases) {
    let pool = databases.pool().await;
    let (adam, eve) = (Uuid::parse_str(ADAM).unwrap(), Uuid::new_v4());
    previous_release(&pool, adam, eve).await;

    let store = Arc::new(Store::open(pool.clone()).await.unwrap());
    let keyring = Keyring::from_env(&format!("1:{}", STANDARD.encode([7; 32]))).unwrap();
    let secrets = Arc::new(
        DatabaseSecretStore::open(pool.clone(), keyring)
            .await
            .unwrap(),
    );
    let environment = Environment::new(store.clone(), secrets.clone(), store.clone(), links());
    assert_eq!(environment.adopt_credentials().await.unwrap(), 2);
    assert_eq!(environment.adopt_credentials().await.unwrap(), 0);
    assert!(
        secrets
            .legacy_credentials(UserId(adam))
            .await
            .unwrap()
            .is_empty()
    );

    let owner: User = store.get(UserId(adam)).await.unwrap().unwrap();
    let adopted: Vec<(String, bool, bool, usize)> = environment
        .list(&owner)
        .await
        .unwrap()
        .into_iter()
        .map(|view| {
            (
                view.entry.target.to_string(),
                view.entry.is_secret(),
                view.entry.apply_to_new,
                view.sandboxes.len(),
            )
        })
        .collect();
    assert_eq!(
        adopted,
        [
            ("variable GH_TOKEN".to_owned(), true, true, 2),
            ("variable SHORT".to_owned(), true, true, 2)
        ]
    );

    let engine = Arc::new(FakeEngine::default());
    let theirs: Vec<Sandbox> = SandboxRepo::list(&*store, UserId(adam)).await.unwrap();
    for sandbox in &theirs {
        engine
            .create(
                &sandbox.id,
                &SandboxSpec {
                    owner: owner.handle.clone(),
                    user: owner.id,
                    name: sandbox.name.clone(),
                    image: "base".to_owned(),
                    resources: catalog().size(None).unwrap().resources,
                },
            )
            .await
            .unwrap();
    }
    let sandboxes = Sandboxes::new(
        store.clone(),
        store.clone(),
        engine.clone(),
        Arc::new(FakeRegistry::default()),
        environment,
        catalog(),
        Limits {
            max_sandboxes_per_user: 5,
            wake_deadline: Duration::from_secs(1),
            idle: corral_core::fakes::IDLE,
        },
    );
    for sandbox in &theirs {
        sandboxes.wake(&owner, &sandbox.name).await.unwrap();
        assert_eq!(
            exposed(&engine.env_of(&sandbox.id).unwrap()),
            [
                "GH_TOKEN=ghp_0123456789abcdefghijklmnopqrstuvwxyz",
                "SHORT=hunter2"
            ]
        );
    }
}

#[tokio::test]
async fn sqlite_upgrades_keeping_every_credential() {
    upgrades_keeping_every_credential(Databases::sqlite()).await;
}

#[tokio::test]
async fn postgres_upgrades_keeping_every_credential() {
    upgrades_keeping_every_credential(Databases::postgres().await).await;
}
