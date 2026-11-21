use async_trait::async_trait;
use chrono::Utc;
use corral_core::Result;
use corral_core::domain::{CredentialName, UserId};
use corral_core::ports::SecretBlobRepo;
use sqlx::Row;
use sqlx::sqlite::SqlitePool;

use crate::decode::parsed;
use crate::error::db_error;

pub(crate) struct SecretBlobs {
    pub(crate) pool: SqlitePool,
}

#[async_trait]
impl SecretBlobRepo for SecretBlobs {
    async fn put(&self, user: UserId, name: &CredentialName, blob: Vec<u8>) -> Result<()> {
        sqlx::query(
            "insert into credential_values (user_id, name, blob, updated_at) \
             values (?, ?, ?, ?) \
             on conflict (user_id, name) do update \
             set blob = excluded.blob, updated_at = excluded.updated_at",
        )
        .bind(user.0.hyphenated())
        .bind(name.as_str())
        .bind(blob)
        .bind(Utc::now())
        .execute(&self.pool)
        .await
        .map_err(db_error)?;
        Ok(())
    }

    async fn delete(&self, user: UserId, name: &CredentialName) -> Result<()> {
        sqlx::query("delete from credential_values where user_id = ? and name = ?")
            .bind(user.0.hyphenated())
            .bind(name.as_str())
            .execute(&self.pool)
            .await
            .map_err(db_error)?;
        Ok(())
    }

    async fn load_all(&self, user: UserId) -> Result<Vec<(CredentialName, Vec<u8>)>> {
        sqlx::query("select name, blob from credential_values where user_id = ? order by name")
            .bind(user.0.hyphenated())
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
