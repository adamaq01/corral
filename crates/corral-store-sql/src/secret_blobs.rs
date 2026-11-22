use async_trait::async_trait;
use chrono::Utc;
use corral_core::Result;
use corral_core::domain::{CredentialName, UserId};
use corral_core::ports::SecretBlobRepo;

use crate::Store;
use crate::error::db_error;
use crate::stored::parsed;

#[async_trait]
impl SecretBlobRepo for Store {
    async fn put(&self, user: UserId, name: &CredentialName, blob: Vec<u8>) -> Result<()> {
        on_pool!(self, |pool| {
            sqlx::query(
                "insert into credential_values (user_id, name, blob, updated_at) \
                 values ($1, $2, $3, $4) \
                 on conflict (user_id, name) do update \
                 set blob = excluded.blob, updated_at = excluded.updated_at",
            )
            .bind(user.0)
            .bind(name.as_str())
            .bind(&blob)
            .bind(Utc::now())
            .execute(pool)
            .await
            .map(|_| ())
        })
        .map_err(db_error)
    }

    async fn delete(&self, user: UserId, name: &CredentialName) -> Result<()> {
        on_pool!(self, |pool| {
            sqlx::query("delete from credential_values where user_id = $1 and name = $2")
                .bind(user.0)
                .bind(name.as_str())
                .execute(pool)
                .await
                .map(|_| ())
        })
        .map_err(db_error)
    }

    async fn load_all(&self, user: UserId) -> Result<Vec<(CredentialName, Vec<u8>)>> {
        let rows: Vec<(String, Vec<u8>)> = on_pool!(self, |pool| {
            sqlx::query_as(
                "select name, blob from credential_values where user_id = $1 order by name",
            )
            .bind(user.0)
            .fetch_all(pool)
            .await
        })
        .map_err(db_error)?;
        rows.into_iter()
            .map(|(name, blob)| {
                Ok((
                    parsed("credential name", CredentialName::parse(&name))?,
                    blob,
                ))
            })
            .collect()
    }
}
