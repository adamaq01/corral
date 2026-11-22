use std::sync::atomic::{AtomicUsize, Ordering};

use tempfile::TempDir;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use testcontainers_modules::testcontainers::{ContainerAsync, ImageExt};

use crate::Pool;

pub struct Sqlite {
    dir: TempDir,
}

impl Sqlite {
    #[must_use]
    pub fn new() -> Self {
        Self {
            dir: tempfile::tempdir().expect("temp dir"),
        }
    }

    #[must_use]
    pub fn url(&self) -> String {
        format!("sqlite://{}", self.dir.path().join("corral.db").display())
    }

    pub async fn pool(&self) -> Pool {
        Pool::connect(&self.url()).await.expect("connect to sqlite")
    }
}

impl Default for Sqlite {
    fn default() -> Self {
        Self::new()
    }
}

pub struct PostgresServer {
    _container: ContainerAsync<Postgres>,
    base: String,
    databases: AtomicUsize,
}

impl PostgresServer {
    /// Starts `postgres:19` and prints its version, so a test log shows the server it ran on.
    pub async fn start() -> Self {
        let container = Postgres::default()
            .with_tag("19")
            .start()
            .await
            .expect("start postgres");
        let base = format!(
            "postgres://postgres:postgres@{}:{}",
            container.get_host().await.expect("postgres host"),
            container
                .get_host_port_ipv4(5432)
                .await
                .expect("postgres port")
        );
        let server = Self {
            _container: container,
            base,
            databases: AtomicUsize::new(0),
        };
        let Pool::Postgres(admin) = Pool::connect(&server.admin_url()).await.expect("connect")
        else {
            unreachable!("a postgres url opens a postgres pool");
        };
        let version: String = sqlx::query_scalar("select version()")
            .fetch_one(&admin)
            .await
            .expect("postgres version");
        println!("testing against {version}");
        server
    }

    fn admin_url(&self) -> String {
        format!("{}/postgres", self.base)
    }

    /// The url of a new, empty database.
    pub async fn url(&self) -> String {
        let name = format!("test_{}", self.databases.fetch_add(1, Ordering::SeqCst));
        let Pool::Postgres(admin) = Pool::connect(&self.admin_url()).await.expect("connect") else {
            unreachable!("a postgres url opens a postgres pool");
        };
        sqlx::query(sqlx::AssertSqlSafe(format!("create database {name}")))
            .execute(&admin)
            .await
            .expect("create database");
        format!("{}/{name}", self.base)
    }

    pub async fn pool(&self) -> Pool {
        Pool::connect(&self.url().await)
            .await
            .expect("connect to postgres")
    }
}
