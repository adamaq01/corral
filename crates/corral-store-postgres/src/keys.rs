use async_trait::async_trait;
use corral_core::Result;
use corral_core::domain::{SshKey, UserId};
use corral_core::ports::KeyRepo;
use sqlx::Row;
use sqlx::postgres::{PgPool, PgRow};
use uuid::Uuid;

use crate::error::db_error;

pub(crate) struct Keys {
    pub(crate) pool: PgPool,
}

fn from_row(row: &PgRow) -> sqlx::Result<SshKey> {
    Ok(SshKey {
        id: row.try_get("id")?,
        user_id: UserId(row.try_get("user_id")?),
        label: row.try_get("label")?,
        public_key: row.try_get("public_key")?,
        fingerprint: row.try_get("fingerprint")?,
        created_at: row.try_get("created_at")?,
    })
}

#[async_trait]
impl KeyRepo for Keys {
    async fn list(&self, user: UserId) -> Result<Vec<SshKey>> {
        sqlx::query(
            "select id, user_id, label, public_key, fingerprint, created_at \
             from ssh_keys where user_id = $1 order by created_at, id",
        )
        .bind(user.0)
        .try_map(|row| from_row(&row))
        .fetch_all(&self.pool)
        .await
        .map_err(db_error)
    }

    async fn insert(&self, key: &SshKey) -> Result<()> {
        sqlx::query(
            "insert into ssh_keys (id, user_id, label, public_key, fingerprint, created_at) \
             values ($1, $2, $3, $4, $5, $6)",
        )
        .bind(key.id)
        .bind(key.user_id.0)
        .bind(&key.label)
        .bind(&key.public_key)
        .bind(&key.fingerprint)
        .bind(key.created_at)
        .execute(&self.pool)
        .await
        .map_err(db_error)?;
        Ok(())
    }

    async fn delete(&self, user: UserId, id: Uuid) -> Result<bool> {
        let deleted = sqlx::query("delete from ssh_keys where user_id = $1 and id = $2")
            .bind(user.0)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(db_error)?;
        Ok(deleted.rows_affected() > 0)
    }

    async fn by_public_key(&self, public_key: &str) -> Result<Option<SshKey>> {
        sqlx::query(
            "select id, user_id, label, public_key, fingerprint, created_at \
             from ssh_keys where public_key = $1",
        )
        .bind(public_key)
        .try_map(|row| from_row(&row))
        .fetch_optional(&self.pool)
        .await
        .map_err(db_error)
    }
}
