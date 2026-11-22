use std::sync::Mutex;

use tempfile::TempDir;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use testcontainers_modules::testcontainers::{ContainerAsync, ImageExt};

use crate::Pool;

/// Hands out new, empty databases of one kind, and removes them when dropped.
#[allow(clippy::large_enum_variant)]
pub enum Databases {
    Sqlite(Mutex<Vec<TempDir>>),
    Postgres {
        _container: ContainerAsync<Postgres>,
        base: String,
        created: Mutex<usize>,
    },
}

impl Databases {
    #[must_use]
    pub fn sqlite() -> Self {
        Self::Sqlite(Mutex::default())
    }

    /// Starts `postgres:19` and prints its version, so a test log shows the server it ran on.
    pub async fn postgres() -> Self {
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
        let admin = sqlx::PgPool::connect(&format!("{base}/postgres"))
            .await
            .expect("connect to postgres");
        let version: String = sqlx::query_scalar("select version()")
            .fetch_one(&admin)
            .await
            .expect("postgres version");
        println!("testing against {version}");
        Self::Postgres {
            _container: container,
            base,
            created: Mutex::new(0),
        }
    }

    pub async fn url(&self) -> String {
        match self {
            Self::Sqlite(dirs) => {
                let dir = tempfile::tempdir().expect("temp dir");
                let url = format!("sqlite://{}", dir.path().join("corral.db").display());
                dirs.lock().expect("sqlite dirs").push(dir);
                url
            }
            Self::Postgres { base, created, .. } => {
                let name = {
                    let mut created = created.lock().expect("database count");
                    *created += 1;
                    format!("test_{created}")
                };
                let admin = sqlx::PgPool::connect(&format!("{base}/postgres"))
                    .await
                    .expect("connect to postgres");
                sqlx::query(sqlx::AssertSqlSafe(format!("create database {name}")))
                    .execute(&admin)
                    .await
                    .expect("create database");
                format!("{base}/{name}")
            }
        }
    }

    pub async fn pool(&self) -> Pool {
        Pool::connect(&self.url().await)
            .await
            .expect("connect to the test database")
    }
}
