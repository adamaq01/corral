use async_trait::async_trait;
use chrono::Utc;
use corral_core::Result;
use corral_core::domain::{CredentialName, UserId};
use corral_core::ports::CredentialRepo;

use crate::Store;
use crate::error::db_error;
use crate::stored::parsed;

#[async_trait]
impl CredentialRepo for Store {
    async fn list(&self, user: UserId) -> Result<Vec<CredentialName>> {
        let names: Vec<String> = on_pool!(self, |pool| {
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

    async fn upsert(&self, user: UserId, name: &CredentialName) -> Result<()> {
        on_pool!(self, |pool| {
            sqlx::query(
                "insert into credentials (user_id, name, created_at, updated_at) \
                 values ($1, $2, $3, $3) \
                 on conflict (user_id, name) do update set updated_at = excluded.updated_at",
            )
            .bind(user.0)
            .bind(name.as_str())
            .bind(Utc::now())
            .execute(pool)
            .await
            .map(|_| ())
        })
        .map_err(db_error)
    }

    async fn delete(&self, user: UserId, name: &CredentialName) -> Result<bool> {
        let deleted = on_pool!(self, |pool| {
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
}
