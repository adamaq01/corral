use async_trait::async_trait;
use chrono::Utc;
use corral_core::Result;
use corral_core::domain::{CredentialName, UserId};
use corral_core::ports::SecretBlobRepo;
use sqlx::Row;
use sqlx::postgres::PgPool;

use crate::decode::parsed;
use crate::error::db_error;

pub(crate) struct SecretBlobs {
    pub(crate) pool: PgPool,
}

#[async_trait]
impl SecretBlobRepo for SecretBlobs {
    async fn put(&self, user: UserId, name: &CredentialName, blob: Vec<u8>) -> Result<()> {
        sqlx::query(
            "insert into credential_values (user_id, name, blob, updated_at) \
             values ($1, $2, $3, $4) \
             on conflict (user_id, name) do update \
             set blob = excluded.blob, updated_at = excluded.updated_at",
        )
        .bind(user.0)
        .bind(name.as_str())
        .bind(blob)
        .bind(Utc::now())
        .execute(&self.pool)
        .await
        .map_err(db_error)?;
        Ok(())
    }

    async fn delete(&self, user: UserId, name: &CredentialName) -> Result<()> {
        sqlx::query("delete from credential_values where user_id = $1 and name = $2")
            .bind(user.0)
            .bind(name.as_str())
            .execute(&self.pool)
            .await
            .map_err(db_error)?;
        Ok(())
    }

    async fn load_all(&self, user: UserId) -> Result<Vec<(CredentialName, Vec<u8>)>> {
        sqlx::query("select name, blob from credential_values where user_id = $1 order by name")
            .bind(user.0)
            .try_map(|row| {
                Ok((
                    parsed(&row, "name", CredentialName::parse)?,
                    row.try_get("blob")?,
                ))
            })
            .fetch_all(&self.pool)
            .await
            .map_err(db_error)
    }
}
