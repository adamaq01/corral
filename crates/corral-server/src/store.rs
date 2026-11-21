use std::sync::Arc;

use anyhow::{Result, bail};
use corral_core::ports::{
    CredentialRepo, KeyRepo, LocalAccountRepo, SandboxRepo, SecretBlobRepo, UserRepo,
};
use corral_store_postgres::PgStore;
use corral_store_sqlite::SqliteStore;

use crate::env;

const DEFAULT_DATABASE_URL: &str = "sqlite:///var/lib/corral/corral.db";

pub enum Database {
    Sqlite(String),
    Postgres(String),
}

impl Database {
    pub fn from_env() -> Result<Self> {
        Self::from_url(env::var("DATABASE_URL").as_deref())
    }

    fn from_url(url: Option<&str>) -> Result<Self> {
        let url = url.unwrap_or(DEFAULT_DATABASE_URL);
        match url.split_once(':').map(|(scheme, _)| scheme) {
            Some("sqlite") => Ok(Self::Sqlite(url.to_owned())),
            Some("postgres" | "postgresql") => Ok(Self::Postgres(url.to_owned())),
            Some(scheme) => {
                bail!("DATABASE_URL scheme {scheme:?} is not supported: use sqlite: or postgres:")
            }
            None => bail!("DATABASE_URL has no scheme: use sqlite: or postgres:"),
        }
    }
}

pub struct Repos {
    pub users: Arc<dyn UserRepo>,
    pub keys: Arc<dyn KeyRepo>,
    pub sandboxes: Arc<dyn SandboxRepo>,
    pub credentials: Arc<dyn CredentialRepo>,
    pub secret_blobs: Arc<dyn SecretBlobRepo>,
    pub local_accounts: Arc<dyn LocalAccountRepo>,
}

impl Repos {
    pub async fn connect(database: &Database) -> Result<Self> {
        Ok(match database {
            Database::Sqlite(url) => {
                let store = SqliteStore::connect(url).await?;
                Self {
                    users: store.users(),
                    keys: store.keys(),
                    sandboxes: store.sandboxes(),
                    credentials: store.credentials(),
                    secret_blobs: store.secret_blobs(),
                    local_accounts: store.local_accounts(),
                }
            }
            Database::Postgres(url) => {
                let store = PgStore::connect(url).await?;
                Self {
                    users: store.users(),
                    keys: store.keys(),
                    sandboxes: store.sandboxes(),
                    credentials: store.credentials(),
                    secret_blobs: store.secret_blobs(),
                    local_accounts: store.local_accounts(),
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::Database;

    #[test]
    fn database_url_scheme_selects_store() {
        assert!(matches!(
            Database::from_url(None).unwrap(),
            Database::Sqlite(url) if url == "sqlite:///var/lib/corral/corral.db"
        ));
        assert!(matches!(
            Database::from_url(Some("sqlite:///tmp/c.db")).unwrap(),
            Database::Sqlite(url) if url == "sqlite:///tmp/c.db"
        ));
        for postgres in ["postgres://u:p@db/corral", "postgresql://u:p@db/corral"] {
            assert!(matches!(
                Database::from_url(Some(postgres)).unwrap(),
                Database::Postgres(url) if url == postgres
            ));
        }
        for unsupported in ["mysql://u:secret-pw@db/corral", "corral.db"] {
            let message = format!("{:#}", Database::from_url(Some(unsupported)).err().unwrap());
            assert!(message.contains("DATABASE_URL"), "{message}");
            assert!(!message.contains("secret-pw"), "{message}");
        }
    }
}
