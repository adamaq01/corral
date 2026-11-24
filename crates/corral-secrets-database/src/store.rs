use std::collections::BTreeMap;

use async_trait::async_trait;
use corral_core::domain::{SecretId, UserId, VariableName};
use corral_core::ports::{SecretStore, VariableMap};
use corral_core::{Error, Result};
use corral_sql::{Migrations, Pool, db_error, on_pool, parsed};
use secrecy::{ExposeSecret, SecretSlice, SecretString};
use uuid::Uuid;

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

    fn unseal(&self, aad: &[u8], sealed: &[u8]) -> Result<Vec<u8>> {
        Sealed::from_bytes(sealed).and_then(|sealed| open(&self.keyring, aad, &sealed))
    }

    /// Keeps a credential the way this store kept them before entries existed.
    #[cfg(feature = "testing")]
    pub async fn keep_legacy(&self, owner: UserId, name: &VariableName, value: &str) {
        let sealed = seal(&self.keyring, &legacy_aad(owner, name), value.as_bytes()).to_bytes();
        on_pool!(self.pool, |pool| {
            sqlx::query("insert into credentials (user_id, name, sealed) values ($1, $2, $3)")
                .bind(owner.0)
                .bind(name.as_str())
                .bind(&sealed)
                .execute(pool)
                .await
                .map(drop)
        })
        .expect("keep a legacy credential");
    }
}

// Each value is sealed to its owner and id, so a row moved to another fails to open.
fn aad(owner: UserId, id: SecretId) -> Vec<u8> {
    format!("{}/secret/{}", owner.0, id.0).into_bytes()
}

fn legacy_aad(owner: UserId, name: &VariableName) -> Vec<u8> {
    format!("{}/{name}", owner.0).into_bytes()
}

#[async_trait]
impl SecretStore for DatabaseSecretStore {
    async fn put(&self, owner: UserId, id: SecretId, value: &SecretSlice<u8>) -> Result<()> {
        let sealed = seal(&self.keyring, &aad(owner, id), value.expose_secret()).to_bytes();
        on_pool!(self.pool, |pool| {
            sqlx::query(
                "insert into secrets (user_id, id, sealed) values ($1, $2, $3) \
                 on conflict (user_id, id) do update set sealed = excluded.sealed",
            )
            .bind(owner.0)
            .bind(id.0)
            .bind(&sealed)
            .execute(pool)
            .await
            .map(drop)
        })
        .map_err(db_error)
    }

    async fn get(
        &self,
        owner: UserId,
        ids: &[SecretId],
    ) -> Result<BTreeMap<SecretId, SecretSlice<u8>>> {
        let rows: Vec<(Uuid, Vec<u8>)> = on_pool!(self.pool, |pool| {
            sqlx::query_as("select id, sealed from secrets where user_id = $1")
                .bind(owner.0)
                .fetch_all(pool)
                .await
        })
        .map_err(db_error)?;
        rows.into_iter()
            .map(|(id, sealed)| (SecretId(id), sealed))
            .filter(|(id, _)| ids.contains(id))
            .map(|(id, sealed)| {
                let value = self
                    .unseal(&aad(owner, id), &sealed)
                    .map_err(|err| Error::internal(format!("secret {}", id.0), err))?;
                Ok((id, SecretSlice::from(value)))
            })
            .collect()
    }

    async fn remove(&self, owner: UserId, id: SecretId) -> Result<bool> {
        let deleted = on_pool!(self.pool, |pool| {
            sqlx::query("delete from secrets where user_id = $1 and id = $2")
                .bind(owner.0)
                .bind(id.0)
                .execute(pool)
                .await
                .map(|done| done.rows_affected())
        })
        .map_err(db_error)?;
        Ok(deleted > 0)
    }

    async fn remove_all(&self, owner: UserId) -> Result<()> {
        self.forget_legacy_credentials(owner).await?;
        on_pool!(self.pool, |pool| {
            sqlx::query("delete from secrets where user_id = $1")
                .bind(owner.0)
                .execute(pool)
                .await
                .map(drop)
        })
        .map_err(db_error)
    }

    async fn legacy_credentials(&self, owner: UserId) -> Result<VariableMap> {
        let rows: Vec<(String, Vec<u8>)> = on_pool!(self.pool, |pool| {
            sqlx::query_as("select name, sealed from credentials where user_id = $1")
                .bind(owner.0)
                .fetch_all(pool)
                .await
        })
        .map_err(db_error)?;
        rows.into_iter()
            .map(|(name, sealed)| {
                let name = parsed("credential name", VariableName::parse(&name))?;
                let value = self
                    .unseal(&legacy_aad(owner, &name), &sealed)
                    .and_then(|plaintext| {
                        String::from_utf8(plaintext)
                            .map_err(|_| Error::internal("sealed value", "not UTF-8"))
                    })
                    .map_err(|err| Error::internal(format!("credential {name}"), err))?;
                Ok((name, SecretString::from(value)))
            })
            .collect()
    }

    async fn forget_legacy_credentials(&self, owner: UserId) -> Result<()> {
        on_pool!(self.pool, |pool| {
            sqlx::query("delete from credentials where user_id = $1")
                .bind(owner.0)
                .execute(pool)
                .await
                .map(drop)
        })
        .map_err(db_error)
    }
}
