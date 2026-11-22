use async_trait::async_trait;
use corral_core::Result;
use corral_core::domain::{SshKey, SshPublicKey, UserId};
use corral_core::ports::KeyRepo;
use sqlx::Row;
use sqlx::sqlite::{SqlitePool, SqliteRow};
use uuid::Uuid;

use crate::decode::{parsed, uuid};
use crate::error::db_error;

pub(crate) struct Keys {
    pub(crate) pool: SqlitePool,
}

fn from_row(row: &SqliteRow) -> sqlx::Result<SshKey> {
    Ok(SshKey {
        id: uuid(row, "id")?,
        user_id: UserId(uuid(row, "user_id")?),
        label: row.try_get("label")?,
        public_key: parsed(row, "public_key", SshPublicKey::parse)?,
        created_at: row.try_get("created_at")?,
    })
}

#[async_trait]
impl KeyRepo for Keys {
    async fn list(&self, user: UserId) -> Result<Vec<SshKey>> {
        sqlx::query(
            "select id, user_id, label, public_key, fingerprint, created_at \
             from ssh_keys where user_id = ? order by created_at, id",
        )
        .bind(user.0.hyphenated())
        .try_map(|row| from_row(&row))
        .fetch_all(&self.pool)
        .await
        .map_err(db_error)
    }

    async fn insert(&self, key: &SshKey) -> Result<()> {
        sqlx::query(
            "insert into ssh_keys (id, user_id, label, public_key, fingerprint, created_at) \
             values (?, ?, ?, ?, ?, ?)",
        )
        .bind(key.id.hyphenated())
        .bind(key.user_id.0.hyphenated())
        .bind(&key.label)
        .bind(key.public_key.as_str())
        .bind(key.public_key.fingerprint())
        .bind(key.created_at)
        .execute(&self.pool)
        .await
        .map_err(db_error)?;
        Ok(())
    }

    async fn delete(&self, user: UserId, id: Uuid) -> Result<bool> {
        let deleted = sqlx::query("delete from ssh_keys where user_id = ? and id = ?")
            .bind(user.0.hyphenated())
            .bind(id.hyphenated())
            .execute(&self.pool)
            .await
            .map_err(db_error)?;
        Ok(deleted.rows_affected() > 0)
    }

    async fn by_public_key(&self, public_key: &SshPublicKey) -> Result<Option<SshKey>> {
        sqlx::query(
            "select id, user_id, label, public_key, fingerprint, created_at \
             from ssh_keys where public_key = ?",
        )
        .bind(public_key.as_str())
        .try_map(|row| from_row(&row))
        .fetch_optional(&self.pool)
        .await
        .map_err(db_error)
    }
}
