use async_trait::async_trait;
use chrono::Utc;
use corral_core::Result;
use corral_core::domain::{CredentialName, UserId};
use corral_core::ports::CredentialRepo;
use sqlx::sqlite::SqlitePool;

use crate::decode::parsed;
use crate::error::db_error;

pub(crate) struct Credentials {
    pub(crate) pool: SqlitePool,
}

#[async_trait]
impl CredentialRepo for Credentials {
    async fn list(&self, user: UserId) -> Result<Vec<CredentialName>> {
        sqlx::query("select name from credentials where user_id = ? order by name")
            .bind(user.0.hyphenated())
            .try_map(|row| parsed(&row, "name", CredentialName::parse))
            .fetch_all(&self.pool)
            .await
            .map_err(db_error)
    }

    async fn upsert(&self, user: UserId, name: &CredentialName) -> Result<()> {
        sqlx::query(
            "insert into credentials (user_id, name, created_at, updated_at) \
             values (?1, ?2, ?3, ?3) \
             on conflict (user_id, name) do update set updated_at = excluded.updated_at",
        )
        .bind(user.0.hyphenated())
        .bind(name.as_str())
        .bind(Utc::now())
        .execute(&self.pool)
        .await
        .map_err(db_error)?;
        Ok(())
    }

    async fn delete(&self, user: UserId, name: &CredentialName) -> Result<bool> {
        let deleted = sqlx::query("delete from credentials where user_id = ? and name = ?")
            .bind(user.0.hyphenated())
            .bind(name.as_str())
            .execute(&self.pool)
            .await
            .map_err(db_error)?;
        Ok(deleted.rows_affected() > 0)
    }
}
