// Expands the body once per driver, so each query compiles against that driver's native types.
// SQLite binds `$N` placeholders by number, so one SQL text serves both.
macro_rules! on_pool {
    ($store:expr, |$pool:ident| $body:expr) => {
        match &$store.pool {
            crate::Pool::Sqlite($pool) => $body,
            crate::Pool::Postgres($pool) => $body,
        }
    };
}

mod error;
mod keys;
mod local_accounts;
mod sandboxes;
mod secrets;
mod stored;
mod users;

use std::str::FromStr;

use corral_core::{Error, Result};
use sqlx::postgres::PgPool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool};

use crate::error::db_error;
pub use crate::secrets::{DatabaseSecretStore, Keyring};

pub struct Store {
    pool: Pool,
}

enum Pool {
    Sqlite(SqlitePool),
    Postgres(PgPool),
}

impl Store {
    pub async fn connect(url: &str) -> Result<Self> {
        let pool = match url.split_once(':').map(|(scheme, _)| scheme) {
            Some("sqlite") => {
                let options = SqliteConnectOptions::from_str(url)
                    .map_err(db_error)?
                    .create_if_missing(true)
                    .journal_mode(SqliteJournalMode::Wal)
                    .foreign_keys(true);
                let pool = SqlitePool::connect_with(options).await.map_err(db_error)?;
                sqlx::migrate!("migrations/sqlite")
                    .run(&pool)
                    .await
                    .map_err(|err| Error::internal("store migration", err))?;
                Pool::Sqlite(pool)
            }
            Some("postgres" | "postgresql") => {
                let pool = PgPool::connect(url).await.map_err(db_error)?;
                sqlx::migrate!("migrations/postgres")
                    .run(&pool)
                    .await
                    .map_err(|err| Error::internal("store migration", err))?;
                Pool::Postgres(pool)
            }
            Some(scheme) => {
                return Err(Error::Invalid(format!(
                    "database scheme {scheme:?} is not supported: use sqlite: or postgres:"
                )));
            }
            None => {
                return Err(Error::Invalid(
                    "database url has no scheme: use sqlite: or postgres:".to_owned(),
                ));
            }
        };
        Ok(Self { pool })
    }
}
