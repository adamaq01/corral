use async_trait::async_trait;
use chrono::{DateTime, Utc};
use corral_core::domain::{SshKey, SshPublicKey, UserId};
use corral_core::ports::KeyRepo;
use corral_core::{Error, Result};
use uuid::Uuid;

use crate::Store;
use corral_sql::{db_error, on_pool, parsed, unique_as};

#[derive(sqlx::FromRow)]
struct KeyRow {
    id: Uuid,
    user_id: Uuid,
    label: String,
    public_key: String,
    created_at: DateTime<Utc>,
}

impl TryFrom<KeyRow> for SshKey {
    type Error = Error;

    fn try_from(row: KeyRow) -> Result<Self> {
        Ok(Self {
            id: row.id,
            user_id: UserId(row.user_id),
            label: row.label,
            public_key: parsed("public key", SshPublicKey::parse(&row.public_key))?,
            created_at: row.created_at,
        })
    }
}

#[async_trait]
impl KeyRepo for Store {
    async fn list(&self, user: UserId) -> Result<Vec<SshKey>> {
        let rows: Vec<KeyRow> = on_pool!(self.pool, |pool| {
            sqlx::query_as(
                "select id, user_id, label, public_key, created_at \
                 from ssh_keys where user_id = $1 order by created_at, id",
            )
            .bind(user.0)
            .fetch_all(pool)
            .await
        })
        .map_err(db_error)?;
        rows.into_iter().map(SshKey::try_from).collect()
    }

    async fn insert(&self, key: &SshKey) -> Result<()> {
        on_pool!(self.pool, |pool| {
            sqlx::query(
                "insert into ssh_keys (id, user_id, label, public_key, created_at) \
                 values ($1, $2, $3, $4, $5)",
            )
            .bind(key.id)
            .bind(key.user_id.0)
            .bind(&key.label)
            .bind(key.public_key.as_str())
            .bind(key.created_at)
            .execute(pool)
            .await
            .map(|_| ())
        })
        .map_err(unique_as(format!(
            "key {} already registered",
            key.public_key.fingerprint()
        )))
    }

    async fn delete(&self, user: UserId, id: Uuid) -> Result<bool> {
        let deleted = on_pool!(self.pool, |pool| {
            sqlx::query("delete from ssh_keys where user_id = $1 and id = $2")
                .bind(user.0)
                .bind(id)
                .execute(pool)
                .await
                .map(|done| done.rows_affected())
        })
        .map_err(db_error)?;
        Ok(deleted > 0)
    }

    async fn by_public_key(&self, public_key: &SshPublicKey) -> Result<Option<SshKey>> {
        let row: Option<KeyRow> = on_pool!(self.pool, |pool| {
            sqlx::query_as(
                "select id, user_id, label, public_key, created_at \
                 from ssh_keys where public_key = $1",
            )
            .bind(public_key.as_str())
            .fetch_optional(pool)
            .await
        })
        .map_err(db_error)?;
        row.map(SshKey::try_from).transpose()
    }
}
