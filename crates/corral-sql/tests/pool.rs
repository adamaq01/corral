use std::borrow::Cow;
use std::time::Duration;

use corral_core::Error;
use corral_sql::testing::{PostgresServer, Sqlite};
use corral_sql::{Migrations, Pool, on_pool};
use sqlx::SqlSafeStr;
use sqlx::migrate::{Migration, MigrationType, Migrator};

fn set(steps: &[&'static str]) -> Migrator {
    Migrator::with_migrations(
        (1..)
            .zip(steps)
            .map(|(version, sql)| {
                Migration::new(
                    version,
                    Cow::Borrowed("step"),
                    MigrationType::Simple,
                    sql.into_sql_str(),
                    false,
                )
            })
            .collect(),
    )
}

fn module(name: &'static str, steps: &[&'static str]) -> Migrations {
    Migrations::new(name, set(steps), set(steps))
}

const ALPHA_V1: &[&str] = &["create table alpha (id text primary key)"];
const ALPHA_V2: &[&str] = &[
    "create table alpha (id text primary key)",
    "alter table alpha add column label text",
];
const BETA: &[&str] = &["create table beta (id text primary key, alpha text)"];

async fn versions(pool: &Pool, module: &str) -> Vec<i64> {
    let sql = format!("select version from _{module}_migrations order by version");
    on_pool!(pool, |pool| {
        sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
            .fetch_all(pool)
            .await
    })
    .unwrap()
}

async fn execute(pool: &Pool, sql: &'static str) -> Result<u64, sqlx::Error> {
    on_pool!(pool, |pool| sqlx::query(sql)
        .execute(pool)
        .await
        .map(|done| done.rows_affected()))
}

async fn modules_share_one_database(pool: &Pool) {
    for _ in 0..2 {
        pool.migrate(module("alpha", ALPHA_V1)).await.unwrap();
        pool.migrate(module("beta", BETA)).await.unwrap();
    }
    assert_eq!(versions(pool, "alpha").await, [1]);
    assert_eq!(versions(pool, "beta").await, [1]);

    pool.migrate(module("alpha", ALPHA_V2)).await.unwrap();
    pool.migrate(module("beta", BETA)).await.unwrap();
    pool.migrate(module("alpha", ALPHA_V2)).await.unwrap();
    assert_eq!(versions(pool, "alpha").await, [1, 2]);
    assert_eq!(versions(pool, "beta").await, [1]);
    execute(
        pool,
        "insert into alpha (id, label) values ('a', 'labelled')",
    )
    .await
    .unwrap();
    execute(pool, "insert into beta (id, alpha) values ('b', 'a')")
        .await
        .unwrap();

    let edited = module("beta", &["create table beta (id text primary key)"]);
    assert!(matches!(
        unlocked(pool.migrate(edited)).await,
        Err(Error::Internal { .. })
    ));
    let broken = module("gamma", &["create table nonsense ("]);
    match unlocked(pool.migrate(broken)).await {
        Err(error @ Error::Internal { .. }) => {
            assert!(error.to_string().contains("gamma"), "{error}");
        }
        other => panic!("a broken migration ran: {other:?}"),
    }
    unlocked(pool.migrate(module("delta", &["create table delta (id text)"])))
        .await
        .unwrap();
}

async fn unlocked<T>(migration: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(10), migration)
        .await
        .expect("an earlier failed migration left the database locked")
}

#[tokio::test]
async fn sqlite_modules_share_one_database() {
    let sqlite = Sqlite::new();
    modules_share_one_database(&sqlite.pool().await).await;
}

#[tokio::test]
async fn postgres_modules_share_one_database() {
    let postgres = PostgresServer::start().await;
    modules_share_one_database(&postgres.pool().await).await;
}

#[tokio::test]
async fn connect_names_only_the_scheme_it_refuses() {
    for unsupported in ["mysql://u:secret-pw@db/corral", "corral.db"] {
        match Pool::connect(unsupported).await {
            Err(Error::Invalid(message)) => {
                assert!(message.contains("sqlite: or postgres:"), "{message}");
                assert!(!message.contains("secret-pw"), "{message}");
            }
            Err(other) => panic!("{unsupported}: {other:?}"),
            Ok(_) => panic!("{unsupported}: connected"),
        }
    }
}

#[tokio::test]
async fn sqlite_connect_creates_a_wal_database_that_enforces_foreign_keys() {
    let sqlite = Sqlite::new();
    let pool = sqlite.pool().await;
    let Pool::Sqlite(raw) = &pool else {
        panic!("a sqlite url opened {pool:?}");
    };
    let mode: String = sqlx::query_scalar("pragma journal_mode")
        .fetch_one(raw)
        .await
        .unwrap();
    assert_eq!(mode, "wal");

    pool.migrate(module(
        "fk",
        &[
            "create table parent (id text primary key)",
            "create table child (parent text not null references parent)",
        ],
    ))
    .await
    .unwrap();
    assert!(
        execute(&pool, "insert into child (parent) values ('missing')")
            .await
            .is_err()
    );
}
