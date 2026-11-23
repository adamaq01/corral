use std::borrow::Cow;

use chrono::{DateTime, Utc};
use corral_core::domain::{SandboxName, UserId};
use corral_core::ports::{EnvironmentRepo, SandboxRepo};
use corral_sql::testing::Databases;
use corral_sql::{Migrations, Pool, on_pool};
use corral_store_sql::Store;
use sqlx::migrate::Migrator;
use uuid::Uuid;

type Moment = Option<DateTime<Utc>>;

/// The migrations before the environment existed.
const BEFORE_ENVIRONMENT: usize = 5;

fn before(mut migrator: Migrator) -> Migrator {
    migrator.migrations = Cow::Owned(migrator.migrations[..BEFORE_ENVIRONMENT].to_vec());
    migrator
}

fn at(seconds: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(1_800_000_000 + seconds, 0).unwrap()
}

async fn exec(pool: &Pool, sql: &'static str, binds: (Uuid, &str, Option<DateTime<Utc>>)) {
    on_pool!(pool, |pool| {
        sqlx::query(sql)
            .bind(binds.0)
            .bind(binds.1)
            .bind(binds.2)
            .execute(pool)
            .await
            .map(drop)
    })
    .unwrap();
}

/// A database at the schema before the environment, with users who changed credentials and
/// users who never did, keeps every row and hands each user over for adoption.
async fn upgrades_a_database_with_real_rows(databases: Databases) {
    let pool = databases.pool().await;
    pool.migrate(Migrations::new(
        "store",
        before(sqlx::migrate!("migrations/sqlite")),
        before(sqlx::migrate!("migrations/postgres")),
    ))
    .await
    .unwrap();
    let (adam, eve) = (Uuid::new_v4(), Uuid::new_v4());
    for (id, handle, changed) in [(adam, "adam", Some(at(5))), (eve, "eve", None)] {
        exec(
            &pool,
            "insert into users (id, subject, handle, role, status, created_at, \
             credentials_changed_at) values ($1, 'local:' || $2, $2, 'member', 'active', \
             '2027-01-15T08:00:00Z', $3)",
            (id, handle, changed),
        )
        .await;
    }
    for (owner, name, created) in [(adam, "main", 0), (adam, "dev", 1), (eve, "main", 2)] {
        on_pool!(pool, |pool| {
            sqlx::query(
                "insert into sandboxes (id, owner, name, image, size, created_at, started_at) \
                 values ($1, $2, $3, 'base', 'small', $4, $5)",
            )
            .bind(format!("sbx-{}", Uuid::new_v4().simple()))
            .bind(owner)
            .bind(name)
            .bind(at(created - 10))
            .bind(at(1))
            .execute(pool)
            .await
            .map(drop)
        })
        .unwrap();
    }

    let store = Store::open(pool.clone()).await.unwrap();
    let changes: Vec<(String, Moment, Moment)> = {
        let mut changes = Vec::new();
        for owner in [adam, eve] {
            for sandbox in store.list(UserId(owner)).await.unwrap() {
                changes.push((
                    format!(
                        "{}.{}",
                        sandbox.name,
                        if owner == adam { "adam" } else { "eve" }
                    ),
                    sandbox.started_at,
                    sandbox.environment_changed_at,
                ));
            }
        }
        changes
    };
    assert_eq!(
        changes,
        [
            ("main.adam".to_owned(), Some(at(1)), Some(at(5))),
            ("dev.adam".to_owned(), Some(at(1)), Some(at(5))),
            ("main.eve".to_owned(), Some(at(1)), None),
        ]
    );
    assert!(
        store
            .by_name(UserId(adam), &SandboxName::parse("main").unwrap())
            .await
            .unwrap()
            .is_some()
    );
    let mut awaiting = store.awaiting_adoption().await.unwrap();
    awaiting.sort();
    let mut expected = vec![UserId(adam), UserId(eve)];
    expected.sort();
    assert_eq!(awaiting, expected);
    store.adopted(UserId(adam)).await.unwrap();
    assert_eq!(store.awaiting_adoption().await.unwrap(), [UserId(eve)]);

    let reopened = Store::open(pool).await.unwrap();
    assert_eq!(reopened.awaiting_adoption().await.unwrap(), [UserId(eve)]);
}

#[tokio::test]
async fn sqlite_upgrades_a_database_with_real_rows() {
    upgrades_a_database_with_real_rows(Databases::sqlite()).await;
}

#[tokio::test]
async fn postgres_upgrades_a_database_with_real_rows() {
    upgrades_a_database_with_real_rows(Databases::postgres().await).await;
}
