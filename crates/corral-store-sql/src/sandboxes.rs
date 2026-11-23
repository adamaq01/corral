use async_trait::async_trait;
use chrono::{DateTime, Utc};
use corral_core::domain::{ImageKey, Sandbox, SandboxId, SandboxName, SizeKey, UserId};
use corral_core::ports::SandboxRepo;
use corral_core::{Error, Result};
use uuid::Uuid;

use crate::Store;
use corral_sql::{db_error, on_pool, parsed, unique_as};

#[derive(sqlx::FromRow)]
struct SandboxRow {
    id: String,
    owner: Uuid,
    name: String,
    image: String,
    size: String,
    created_at: DateTime<Utc>,
    started_at: Option<DateTime<Utc>>,
}

impl TryFrom<SandboxRow> for Sandbox {
    type Error = Error;

    fn try_from(row: SandboxRow) -> Result<Self> {
        Ok(Self {
            id: SandboxId(row.id),
            owner: UserId(row.owner),
            name: parsed("sandbox name", SandboxName::parse(&row.name))?,
            image: parsed("image key", ImageKey::parse(&row.image))?,
            size: parsed("size key", SizeKey::parse(&row.size))?,
            created_at: row.created_at,
            started_at: row.started_at,
        })
    }
}

#[async_trait]
impl SandboxRepo for Store {
    async fn list(&self, owner: UserId) -> Result<Vec<Sandbox>> {
        let rows: Vec<SandboxRow> = on_pool!(self.pool, |pool| {
            sqlx::query_as(
                "select id, owner, name, image, size, created_at, started_at \
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
        let rows: Vec<SandboxRow> = on_pool!(self.pool, |pool| {
            sqlx::query_as(
                "select id, owner, name, image, size, created_at, started_at \
                 from sandboxes order by created_at, id",
            )
            .fetch_all(pool)
            .await
        })
        .map_err(db_error)?;
        rows.into_iter().map(Sandbox::try_from).collect()
    }

    async fn by_name(&self, owner: UserId, name: &SandboxName) -> Result<Option<Sandbox>> {
        let row: Option<SandboxRow> = on_pool!(self.pool, |pool| {
            sqlx::query_as(
                "select id, owner, name, image, size, created_at, started_at \
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

    async fn insert(&self, sandbox: &Sandbox, max_per_owner: usize) -> Result<()> {
        let max = i64::try_from(max_per_owner).unwrap_or(i64::MAX);
        let admitted = on_pool!(self.pool, |pool| async {
            let mut tx = pool.begin().await?;
            // Writing the owner's row first serializes inserts for one owner, so the count below
            // sees every committed sandbox of theirs.
            sqlx::query("update users set default_sandbox = default_sandbox where id = $1")
                .bind(sandbox.owner.0)
                .execute(&mut *tx)
                .await?;
            sqlx::query(
                "insert into sandboxes (id, owner, name, image, size, created_at) \
                 values ($1, $2, $3, $4, $5, $6)",
            )
            .bind(&sandbox.id.0)
            .bind(sandbox.owner.0)
            .bind(sandbox.name.as_str())
            .bind(sandbox.image.as_str())
            .bind(sandbox.size.as_str())
            .bind(sandbox.created_at)
            .execute(&mut *tx)
            .await?;
            let count: i64 = sqlx::query_scalar("select count(*) from sandboxes where owner = $1")
                .bind(sandbox.owner.0)
                .fetch_one(&mut *tx)
                .await?;
            if count > max {
                return Ok(false);
            }
            tx.commit().await?;
            Ok(true)
        }
        .await)
        .map_err(unique_as(format!(
            "sandbox {} already exists",
            sandbox.name
        )))?;
        if admitted {
            Ok(())
        } else {
            Err(Error::LimitReached(format!(
                "at most {max_per_owner} sandboxes per user"
            )))
        }
    }

    async fn delete(&self, owner: UserId, name: &SandboxName) -> Result<bool> {
        let deleted = on_pool!(self.pool, |pool| {
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

    async fn default_id(&self, owner: UserId) -> Result<Option<SandboxId>> {
        let picked: Option<Option<String>> = on_pool!(self.pool, |pool| {
            sqlx::query_scalar("select default_sandbox from users where id = $1")
                .bind(owner.0)
                .fetch_optional(pool)
                .await
        })
        .map_err(db_error)?;
        Ok(picked.flatten().map(SandboxId))
    }

    async fn set_default(&self, owner: UserId, id: &SandboxId) -> Result<()> {
        on_pool!(self.pool, |pool| {
            sqlx::query(
                "update users set default_sandbox = $2 where id = $1 \
                 and exists (select 1 from sandboxes where id = $2 and owner = $1)",
            )
            .bind(owner.0)
            .bind(&id.0)
            .execute(pool)
            .await
            .map(drop)
        })
        .map_err(db_error)
    }

    async fn record_start(&self, id: &SandboxId, at: DateTime<Utc>) -> Result<()> {
        on_pool!(self.pool, |pool| {
            sqlx::query("update sandboxes set started_at = $2 where id = $1")
                .bind(&id.0)
                .bind(at)
                .execute(pool)
                .await
                .map(drop)
        })
        .map_err(db_error)
    }
}
