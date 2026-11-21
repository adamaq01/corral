use async_trait::async_trait;
use corral_core::domain::{Sandbox, SandboxId, SandboxName, UserId};
use corral_core::ports::SandboxRepo;
use corral_core::{Error, Result};
use sqlx::Row;
use sqlx::sqlite::{SqlitePool, SqliteRow};

use crate::decode::{parsed, uuid};
use crate::error::db_error;

pub(crate) struct Sandboxes {
    pub(crate) pool: SqlitePool,
}

fn from_row(row: &SqliteRow) -> sqlx::Result<Sandbox> {
    Ok(Sandbox {
        id: SandboxId(row.try_get("id")?),
        owner: UserId(uuid(row, "owner")?),
        name: parsed(row, "name", SandboxName::parse)?,
        image: row.try_get("image")?,
        size: row.try_get("size")?,
        is_default: row.try_get("is_default")?,
        created_at: row.try_get("created_at")?,
    })
}

#[async_trait]
impl SandboxRepo for Sandboxes {
    async fn list(&self, owner: UserId) -> Result<Vec<Sandbox>> {
        sqlx::query(
            "select id, owner, name, image, size, is_default, created_at \
             from sandboxes where owner = ? order by created_at, id",
        )
        .bind(owner.0.hyphenated())
        .try_map(|row| from_row(&row))
        .fetch_all(&self.pool)
        .await
        .map_err(db_error)
    }

    async fn list_all(&self) -> Result<Vec<Sandbox>> {
        sqlx::query(
            "select id, owner, name, image, size, is_default, created_at \
             from sandboxes order by created_at, id",
        )
        .try_map(|row| from_row(&row))
        .fetch_all(&self.pool)
        .await
        .map_err(db_error)
    }

    async fn by_name(&self, owner: UserId, name: &SandboxName) -> Result<Option<Sandbox>> {
        sqlx::query(
            "select id, owner, name, image, size, is_default, created_at \
             from sandboxes where owner = ? and name = ?",
        )
        .bind(owner.0.hyphenated())
        .bind(name.as_str())
        .try_map(|row| from_row(&row))
        .fetch_optional(&self.pool)
        .await
        .map_err(db_error)
    }

    async fn insert(&self, sandbox: &Sandbox) -> Result<()> {
        sqlx::query(
            "insert into sandboxes (id, owner, name, image, size, is_default, created_at) \
             values (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&sandbox.id.0)
        .bind(sandbox.owner.0.hyphenated())
        .bind(sandbox.name.as_str())
        .bind(&sandbox.image)
        .bind(&sandbox.size)
        .bind(sandbox.is_default)
        .bind(sandbox.created_at)
        .execute(&self.pool)
        .await
        .map_err(db_error)?;
        Ok(())
    }

    async fn update(&self, sandbox: &Sandbox) -> Result<()> {
        let updated = sqlx::query(
            "update sandboxes set owner = ?, name = ?, image = ?, size = ?, is_default = ?, \
             created_at = ? where id = ?",
        )
        .bind(sandbox.owner.0.hyphenated())
        .bind(sandbox.name.as_str())
        .bind(&sandbox.image)
        .bind(&sandbox.size)
        .bind(sandbox.is_default)
        .bind(sandbox.created_at)
        .bind(&sandbox.id.0)
        .execute(&self.pool)
        .await
        .map_err(db_error)?;
        if updated.rows_affected() == 0 {
            return Err(Error::NotFound(format!("sandbox {}", sandbox.name)));
        }
        Ok(())
    }

    async fn delete(&self, owner: UserId, name: &SandboxName) -> Result<bool> {
        let deleted = sqlx::query("delete from sandboxes where owner = ? and name = ?")
            .bind(owner.0.hyphenated())
            .bind(name.as_str())
            .execute(&self.pool)
            .await
            .map_err(db_error)?;
        Ok(deleted.rows_affected() > 0)
    }
}
