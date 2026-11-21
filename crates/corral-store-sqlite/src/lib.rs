mod credentials;
mod decode;
mod error;
mod keys;
mod local_accounts;
mod sandboxes;
mod secret_blobs;
mod users;

use std::str::FromStr;
use std::sync::Arc;

use corral_core::ports::{
    CredentialRepo, KeyRepo, LocalAccountRepo, SandboxRepo, SecretBlobRepo, UserRepo,
};
use corral_core::{Error, Result};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool};

use credentials::Credentials;
use error::db_error;
use keys::Keys;
use local_accounts::LocalAccounts;
use sandboxes::Sandboxes;
use secret_blobs::SecretBlobs;
use users::Users;

pub struct SqliteStore {
    pool: SqlitePool,
}

impl SqliteStore {
    pub async fn connect(url: &str) -> Result<Self> {
        let options = SqliteConnectOptions::from_str(url)
            .map_err(db_error)?
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .foreign_keys(true);
        let pool = SqlitePool::connect_with(options).await.map_err(db_error)?;
        sqlx::migrate!()
            .run(&pool)
            .await
            .map_err(|err| Error::Store(err.to_string()))?;
        Ok(Self { pool })
    }

    #[must_use]
    pub fn users(&self) -> Arc<dyn UserRepo> {
        Arc::new(Users {
            pool: self.pool.clone(),
        })
    }

    #[must_use]
    pub fn keys(&self) -> Arc<dyn KeyRepo> {
        Arc::new(Keys {
            pool: self.pool.clone(),
        })
    }

    #[must_use]
    pub fn sandboxes(&self) -> Arc<dyn SandboxRepo> {
        Arc::new(Sandboxes {
            pool: self.pool.clone(),
        })
    }

    #[must_use]
    pub fn credentials(&self) -> Arc<dyn CredentialRepo> {
        Arc::new(Credentials {
            pool: self.pool.clone(),
        })
    }

    #[must_use]
    pub fn secret_blobs(&self) -> Arc<dyn SecretBlobRepo> {
        Arc::new(SecretBlobs {
            pool: self.pool.clone(),
        })
    }

    #[must_use]
    pub fn local_accounts(&self) -> Arc<dyn LocalAccountRepo> {
        Arc::new(LocalAccounts {
            pool: self.pool.clone(),
        })
    }
}
