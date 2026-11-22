use async_trait::async_trait;
use corral_core::domain::{CredentialName, UserId};
use corral_core::ports::{CredentialMap, SecretStore};
use corral_core::{Error, Result};
use corral_sql::{Migrations, Pool, db_error, on_pool, parsed};
use secrecy::{ExposeSecret, SecretString};

use crate::Keyring;
use crate::cipher::{Sealed, open, seal};

pub struct DatabaseSecretStore {
    pool: Pool,
    keyring: Keyring,
}

impl DatabaseSecretStore {
    pub async fn open(pool: Pool, keyring: Keyring) -> Result<Self> {
        pool.migrate(Migrations::new(
            "credentials",
            sqlx::migrate!("migrations/sqlite"),
            sqlx::migrate!("migrations/postgres"),
        ))
        .await?;
        Ok(Self { pool, keyring })
    }

    fn unseal(&self, user: UserId, name: &CredentialName, sealed: &[u8]) -> Result<SecretString> {
        Sealed::from_bytes(sealed)
            .and_then(|sealed| open(&self.keyring, &aad(user, name), &sealed))
            .and_then(|plaintext| {
                String::from_utf8(plaintext)
                    .map_err(|_| Error::internal("sealed value", "not UTF-8"))
            })
            .map(SecretString::from)
            .map_err(|err| Error::internal(format!("credential {name}"), err))
    }
}

fn aad(user: UserId, name: &CredentialName) -> Vec<u8> {
    format!("{}/{name}", user.0).into_bytes()
}

#[async_trait]
impl SecretStore for DatabaseSecretStore {
    async fn set(&self, user: UserId, name: &CredentialName, value: SecretString) -> Result<()> {
        let sealed = seal(
            &self.keyring,
            &aad(user, name),
            value.expose_secret().as_bytes(),
        )
        .to_bytes();
        on_pool!(self.pool, |pool| {
            sqlx::query(
                "insert into credentials (user_id, name, sealed) values ($1, $2, $3) \
                 on conflict (user_id, name) do update set sealed = excluded.sealed",
            )
            .bind(user.0)
            .bind(name.as_str())
            .bind(&sealed)
            .execute(pool)
            .await
            .map(|_| ())
        })
        .map_err(db_error)
    }

    async fn remove(&self, user: UserId, name: &CredentialName) -> Result<bool> {
        let deleted = on_pool!(self.pool, |pool| {
            sqlx::query("delete from credentials where user_id = $1 and name = $2")
                .bind(user.0)
                .bind(name.as_str())
                .execute(pool)
                .await
                .map(|done| done.rows_affected())
        })
        .map_err(db_error)?;
        Ok(deleted > 0)
    }

    async fn remove_all(&self, user: UserId) -> Result<()> {
        on_pool!(self.pool, |pool| {
            sqlx::query("delete from credentials where user_id = $1")
                .bind(user.0)
                .execute(pool)
                .await
                .map(|_| ())
        })
        .map_err(db_error)
    }

    async fn names(&self, user: UserId) -> Result<Vec<CredentialName>> {
        let names: Vec<String> = on_pool!(self.pool, |pool| {
            sqlx::query_scalar("select name from credentials where user_id = $1 order by name")
                .bind(user.0)
                .fetch_all(pool)
                .await
        })
        .map_err(db_error)?;
        names
            .iter()
            .map(|name| parsed("credential name", CredentialName::parse(name)))
            .collect()
    }

    async fn load(&self, user: UserId) -> Result<CredentialMap> {
        let rows: Vec<(String, Vec<u8>)> = on_pool!(self.pool, |pool| {
            sqlx::query_as("select name, sealed from credentials where user_id = $1")
                .bind(user.0)
                .fetch_all(pool)
                .await
        })
        .map_err(db_error)?;
        rows.into_iter()
            .map(|(name, sealed)| {
                let name = parsed("credential name", CredentialName::parse(&name))?;
                let value = self.unseal(user, &name, &sealed)?;
                Ok((name, value))
            })
            .collect()
    }
}
