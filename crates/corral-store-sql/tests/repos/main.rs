use std::sync::Arc;

use chrono::{DateTime, TimeDelta, Utc};
use corral_core::contract::repos;
use corral_core::domain::{
    Handle, ImageKey, Role, Sandbox, SandboxId, SandboxName, SizeKey, User, UserId, UserStatus,
};
use corral_core::ports::{SandboxRepo, UserRepo};
use corral_sql::testing::Databases;
use corral_sql::{Pool, on_pool};
use corral_store_sql::Store;
use uuid::Uuid;

async fn meets_every_repo_contract(url: &str) {
    Store::open(Pool::connect(url).await.expect("connect"))
        .await
        .expect("open");
    let store = Arc::new(
        Store::open(Pool::connect(url).await.expect("reconnect"))
            .await
            .expect("reopen a migrated database"),
    );
    repos::every_repo(&*store, &*store, &*store).await;

    let gone = repos::deleting_a_user_leaves_nothing_of_them(&*store, &*store, &*store).await;
    let pool = Pool::connect(url).await.expect("raw connection");
    for table in [
        "users where id",
        "ssh_keys where user_id",
        "sandboxes where owner",
    ] {
        let sql = format!("select count(*) from {table} = $1");
        let left: i64 = on_pool!(pool, |pool| {
            sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
                .bind(gone.id.0)
                .fetch_one(pool)
                .await
        })
        .unwrap();
        assert_eq!(left, 0, "{table}");
    }
}

#[tokio::test]
async fn sqlite_meets_every_repo_contract() {
    let sqlite = Databases::sqlite();
    meets_every_repo_contract(&sqlite.url().await).await;
}

#[tokio::test]
async fn postgres_meets_every_repo_contract() {
    let postgres = Databases::postgres().await;
    meets_every_repo_contract(&postgres.url().await).await;
}

async fn a_scheduled_deletion_outlives_the_store(url: &str) {
    let owner = user();
    let sandbox = Sandbox {
        id: SandboxId("sbx-scheduled".to_owned()),
        owner: owner.id,
        name: SandboxName::parse("main").unwrap(),
        image: ImageKey::parse("default").unwrap(),
        size: SizeKey::parse("small").unwrap(),
        created_at: Utc::now(),
        started_at: None,
        delete_at: None,
    };
    let at = DateTime::from_timestamp(1_800_000_000, 0).unwrap();
    {
        let store = Store::open(Pool::connect(url).await.unwrap())
            .await
            .unwrap();
        UserRepo::insert(&store, &owner).await.unwrap();
        SandboxRepo::insert(&store, &sandbox, 5).await.unwrap();
        store.set_delete_at(&sandbox.id, Some(at)).await.unwrap();
    }
    let reopened = Store::open(Pool::connect(url).await.unwrap())
        .await
        .unwrap();
    let due: Vec<(SandboxId, Option<DateTime<Utc>>)> = reopened
        .due_for_deletion(at)
        .await
        .unwrap()
        .into_iter()
        .map(|sandbox| (sandbox.id, sandbox.delete_at))
        .collect();
    assert_eq!(due, [(sandbox.id, Some(at))]);
}

#[tokio::test]
async fn sqlite_keeps_a_scheduled_deletion_across_a_restart() {
    let sqlite = Databases::sqlite();
    a_scheduled_deletion_outlives_the_store(&sqlite.url().await).await;
}

#[tokio::test]
async fn postgres_keeps_a_scheduled_deletion_across_a_restart() {
    let postgres = Databases::postgres().await;
    a_scheduled_deletion_outlives_the_store(&postgres.url().await).await;
}

fn user() -> User {
    User {
        id: UserId(Uuid::new_v4()),
        subject: "subject-adam".to_owned(),
        handle: Handle::parse("adam").unwrap(),
        email: None,
        picture: None,
        role: Role::Member,
        status: UserStatus::Active,
        created_at: Utc::now(),
    }
}

#[tokio::test]
async fn sqlite_sub_second_timestamps_round_trip_in_order() {
    let sqlite = Databases::sqlite();
    let store = Store::open(sqlite.pool().await).await.expect("open");
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
        image: ImageKey::parse("default").unwrap(),
        size: SizeKey::parse("small").unwrap(),
        created_at: second + offset,
        started_at: None,
        delete_at: None,
    })
    .collect();

    for sandbox in chronological.iter().rev() {
        SandboxRepo::insert(&store, sandbox, 5).await.unwrap();
    }
    assert_eq!(
        SandboxRepo::list(&store, owner.id).await.unwrap(),
        chronological
    );
}
