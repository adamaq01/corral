use std::str::FromStr;

use corral_core::{Error, Result};
use sqlx::postgres::PgPool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool};

use crate::Migrations;
use crate::error::db_error;

/// One database shared by every module that keeps state; each module brings its own tables.
#[derive(Clone, Debug)]
pub enum Pool {
    Sqlite(SqlitePool),
    Postgres(PgPool),
}

impl Pool {
    pub async fn connect(url: &str) -> Result<Self> {
        match url.split_once(':').map(|(scheme, _)| scheme) {
            Some("sqlite") => {
                let options = SqliteConnectOptions::from_str(url)
                    .map_err(db_error)?
                    .create_if_missing(true)
                    .journal_mode(SqliteJournalMode::Wal)
                    .foreign_keys(true);
                let pool = SqlitePool::connect_with(options).await.map_err(db_error)?;
                Ok(Self::Sqlite(pool))
            }
            Some("postgres" | "postgresql") => Ok(Self::Postgres(
                PgPool::connect(url).await.map_err(db_error)?,
            )),
            Some(scheme) => Err(Error::Invalid(format!(
                "database scheme {scheme:?} is not supported: use sqlite: or postgres:"
            ))),
            None => Err(Error::Invalid(
                "database url has no scheme: use sqlite: or postgres:".to_owned(),
            )),
        }
    }

    /// Applies the module's migrations that this database has not seen yet.
    pub async fn migrate(&self, migrations: Migrations) -> Result<()> {
        let module = migrations.module;
        let applied = match self {
            Self::Sqlite(pool) => migrations.sqlite().run(pool).await,
            Self::Postgres(pool) => {
                // A failed run returns without releasing its advisory lock, so it runs on a
                // session of its own that closes with it.
                let mut session = pool.acquire().await.map_err(db_error)?.detach();
                migrations.postgres().run(&mut session).await
            }
        };
        applied.map_err(|err| Error::internal(format!("{module} migration"), err))
    }
}
