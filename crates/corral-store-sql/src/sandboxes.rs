use async_trait::async_trait;
use chrono::{DateTime, Utc};
use corral_core::domain::{Sandbox, SandboxId, SandboxName, UserId};
use corral_core::ports::SandboxRepo;
use corral_core::{Error, Result};
use uuid::Uuid;

use crate::Store;
use crate::error::db_error;
use crate::stored::parsed;

#[derive(sqlx::FromRow)]
struct SandboxRow {
    id: String,
    owner: Uuid,
    name: String,
    image: String,
    size: String,
    is_default: bool,
    created_at: DateTime<Utc>,
}

impl TryFrom<SandboxRow> for Sandbox {
    type Error = Error;

    fn try_from(row: SandboxRow) -> Result<Self> {
        Ok(Self {
            id: SandboxId(row.id),
            owner: UserId(row.owner),
            name: parsed("sandbox name", SandboxName::parse(&row.name))?,
            image: row.image,
            size: row.size,
            is_default: row.is_default,
            created_at: row.created_at,
        })
    }
}

#[async_trait]
impl SandboxRepo for Store {
    async fn list(&self, owner: UserId) -> Result<Vec<Sandbox>> {
        let rows: Vec<SandboxRow> = on_pool!(self, |pool| {
            sqlx::query_as(
                "select id, owner, name, image, size, is_default, created_at \
                 from sandboxes where owner = $1 order by created_at, id",
            )
            .bind(owner.0)
            .fetch_all(pool)
            .await
        })
        .map_err(db_error)?;
        rows.into_iter().map(Sandbox::try_from).collect()
    }

    async fn list_all(&self) -> Result<Vec<Sandbox>> {
        let rows: Vec<SandboxRow> = on_pool!(self, |pool| {
            sqlx::query_as(
                "select id, owner, name, image, size, is_default, created_at \
                 from sandboxes order by created_at, id",
            )
            .fetch_all(pool)
            .await
        })
        .map_err(db_error)?;
        rows.into_iter().map(Sandbox::try_from).collect()
    }

    async fn by_name(&self, owner: UserId, name: &SandboxName) -> Result<Option<Sandbox>> {
        let row: Option<SandboxRow> = on_pool!(self, |pool| {
            sqlx::query_as(
                "select id, owner, name, image, size, is_default, created_at \
                 from sandboxes where owner = $1 and name = $2",
            )
            .bind(owner.0)
            .bind(name.as_str())
            .fetch_optional(pool)
            .await
        })
        .map_err(db_error)?;
        row.map(Sandbox::try_from).transpose()
    }

    async fn insert(&self, sandbox: &Sandbox) -> Result<()> {
        on_pool!(self, |pool| {
            sqlx::query(
                "insert into sandboxes (id, owner, name, image, size, is_default, created_at) \
                 values ($1, $2, $3, $4, $5, $6, $7)",
            )
            .bind(&sandbox.id.0)
            .bind(sandbox.owner.0)
            .bind(sandbox.name.as_str())
            .bind(&sandbox.image)
            .bind(&sandbox.size)
            .bind(sandbox.is_default)
            .bind(sandbox.created_at)
            .execute(pool)
            .await
            .map(|_| ())
        })
        .map_err(db_error)
    }

    async fn update(&self, sandbox: &Sandbox) -> Result<()> {
        let updated = on_pool!(self, |pool| {
            sqlx::query(
                "update sandboxes set owner = $1, name = $2, image = $3, size = $4, \
                 is_default = $5, created_at = $6 where id = $7",
            )
            .bind(sandbox.owner.0)
            .bind(sandbox.name.as_str())
            .bind(&sandbox.image)
            .bind(&sandbox.size)
            .bind(sandbox.is_default)
            .bind(sandbox.created_at)
            .bind(&sandbox.id.0)
            .execute(pool)
            .await
            .map(|done| done.rows_affected())
        })
        .map_err(db_error)?;
        if updated == 0 {
            return Err(Error::NotFound(format!("sandbox {}", sandbox.name)));
        }
        Ok(())
    }

    async fn delete(&self, owner: UserId, name: &SandboxName) -> Result<bool> {
        let deleted = on_pool!(self, |pool| {
            sqlx::query("delete from sandboxes where owner = $1 and name = $2")
                .bind(owner.0)
                .bind(name.as_str())
                .execute(pool)
                .await
                .map(|done| done.rows_affected())
        })
        .map_err(db_error)?;
        Ok(deleted > 0)
    }
}
