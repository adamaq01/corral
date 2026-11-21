use chrono::Utc;
use corral_core::contract::repos;
use corral_core::domain::{CredentialName, Handle, Role, User, UserId};
use corral_core::ports::{CredentialRepo, UserRepo};
use corral_store_postgres::PgStore;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::ImageExt;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use uuid::Uuid;

#[tokio::test]
async fn pg_store_meets_every_repo_contract() {
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
    PgStore::connect(&url).await.expect("connect");
    let store = PgStore::connect(&url)
        .await
        .expect("reconnect to a migrated database");
    let users = store.users();

    repos::user_repo(&*users).await;
    repos::key_repo(&*users, &*store.keys()).await;
    repos::sandbox_repo(&*users, &*store.sandboxes()).await;
    repos::credential_repo(&*users, &*store.credentials()).await;
    repos::secret_blob_repo(&*users, &*store.secret_blobs()).await;
    credential_names_list_in_byte_order(&*users, &*store.credentials()).await;
}

async fn credential_names_list_in_byte_order(
    users: &dyn UserRepo,
    credentials: &dyn CredentialRepo,
) {
    let user = User {
        id: UserId(Uuid::new_v4()),
        subject: "subject-collation".to_owned(),
        handle: Handle::parse("collation").unwrap(),
        email: None,
        role: Role::Member,
        enabled: true,
        created_at: Utc::now(),
    };
    users.insert(&user).await.unwrap();
    let mut names: Vec<CredentialName> = ["A_A", "AB", "A1", "A_"]
        .into_iter()
        .map(|name| CredentialName::parse(name).unwrap())
        .collect();
    for name in &names {
        credentials.upsert(user.id, name).await.unwrap();
    }
    names.sort();
    assert_eq!(credentials.list(user.id).await.unwrap(), names);
}
