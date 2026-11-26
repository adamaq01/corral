use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use corral_core::domain::{
    Sandbox, SandboxId, SandboxImage, SandboxName, SizeKey, TokenHash, UserId,
};
use corral_core::ports::{Changed, SandboxRepo};
use corral_core::{Error, Result};
use uuid::Uuid;

use crate::Store;
use corral_sql::{db_error, on_pool, parsed, unique_as};

/// A query for whole sandboxes, `$rest` after its `from`.
macro_rules! select {
    ($rest:literal) => {
        concat!(
            "select id, owner, name, image, size, created_at, started_at, setup_changed_at, \
             delete_at, idle_timeout_secs, keep_awake_until from sandboxes ",
            $rest
        )
    };
}

#[derive(sqlx::FromRow)]
struct SandboxRow {
    id: String,
    owner: Uuid,
    name: String,
    image: String,
    size: String,
    created_at: DateTime<Utc>,
    started_at: Option<DateTime<Utc>>,
    setup_changed_at: Option<DateTime<Utc>>,
    delete_at: Option<DateTime<Utc>>,
    idle_timeout_secs: Option<i64>,
    keep_awake_until: Option<DateTime<Utc>>,
}

impl TryFrom<SandboxRow> for Sandbox {
    type Error = Error;

    fn try_from(row: SandboxRow) -> Result<Self> {
        Ok(Self {
            id: SandboxId(row.id),
            owner: UserId(row.owner),
            name: parsed("sandbox name", SandboxName::parse(&row.name))?,
            image: parsed("image", SandboxImage::parse(&row.image))?,
            size: parsed("size key", SizeKey::parse(&row.size))?,
            created_at: row.created_at,
            started_at: row.started_at,
            setup_changed_at: row.setup_changed_at,
            delete_at: row.delete_at,
            idle_timeout: row
                .idle_timeout_secs
                .map(|secs| u64::try_from(secs).map(Duration::from_secs))
                .transpose()
                .map_err(|err| Error::internal("stored idle timeout", err))?,
            keep_awake_until: row.keep_awake_until,
        })
    }
}

#[async_trait]
impl SandboxRepo for Store {
    async fn list(&self, owner: UserId) -> Result<Vec<Sandbox>> {
        let rows: Vec<SandboxRow> = on_pool!(self.pool, |pool| {
            sqlx::query_as(select!("where owner = $1 order by created_at, id"))
                .bind(owner.0)
                .fetch_all(pool)
                .await
        })
        .map_err(db_error)?;
        rows.into_iter().map(Sandbox::try_from).collect()
    }

    async fn list_all(&self) -> Result<Vec<Sandbox>> {
        let rows: Vec<SandboxRow> = on_pool!(self.pool, |pool| {
            sqlx::query_as(select!("order by created_at, id"))
                .fetch_all(pool)
                .await
        })
        .map_err(db_error)?;
        rows.into_iter().map(Sandbox::try_from).collect()
    }

    async fn by_name(&self, owner: UserId, name: &SandboxName) -> Result<Option<Sandbox>> {
        let row: Option<SandboxRow> = on_pool!(self.pool, |pool| {
            sqlx::query_as(select!("where owner = $1 and name = $2"))
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
            .bind(sandbox.image.to_string())
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

    async fn record_start(
        &self,
        id: &SandboxId,
        at: DateTime<Utc>,
        token: &TokenHash,
    ) -> Result<()> {
        on_pool!(self.pool, |pool| {
            sqlx::query("update sandboxes set started_at = $2, token_hash = $3 where id = $1")
                .bind(&id.0)
                .bind(at)
                .bind(token.as_str())
                .execute(pool)
                .await
                .map(drop)
        })
        .map_err(db_error)
    }

    async fn record_setup_change(&self, which: Changed<'_>, at: DateTime<Utc>) -> Result<()> {
        match which {
            Changed::Sandboxes(ids) => {
                for id in ids {
                    on_pool!(self.pool, |pool| {
                        sqlx::query("update sandboxes set setup_changed_at = $2 where id = $1")
                            .bind(&id.0)
                            .bind(at)
                            .execute(pool)
                            .await
                            .map(drop)
                    })
                    .map_err(db_error)?;
                }
                Ok(())
            }
            Changed::Every => on_pool!(self.pool, |pool| {
                sqlx::query("update sandboxes set setup_changed_at = $1")
                    .bind(at)
                    .execute(pool)
                    .await
                    .map(drop)
            })
            .map_err(db_error),
        }
    }

    async fn revoke_token(&self, id: &SandboxId) -> Result<()> {
        on_pool!(self.pool, |pool| {
            sqlx::query("update sandboxes set token_hash = null where id = $1")
                .bind(&id.0)
                .execute(pool)
                .await
                .map(drop)
        })
        .map_err(db_error)
    }

    async fn by_token(&self, token: &TokenHash) -> Result<Option<Sandbox>> {
        let row: Option<SandboxRow> = on_pool!(self.pool, |pool| {
            sqlx::query_as(select!("where token_hash = $1"))
                .bind(token.as_str())
                .fetch_optional(pool)
                .await
        })
        .map_err(db_error)?;
        row.map(Sandbox::try_from).transpose()
    }

    async fn set_delete_at(&self, id: &SandboxId, at: Option<DateTime<Utc>>) -> Result<()> {
        on_pool!(self.pool, |pool| {
            sqlx::query("update sandboxes set delete_at = $2 where id = $1")
                .bind(&id.0)
                .bind(at)
                .execute(pool)
                .await
                .map(drop)
        })
        .map_err(db_error)
    }

    async fn due_for_deletion(&self, now: DateTime<Utc>) -> Result<Vec<Sandbox>> {
        let rows: Vec<SandboxRow> = on_pool!(self.pool, |pool| {
            sqlx::query_as(select!("where delete_at <= $1 order by created_at, id"))
                .bind(now)
                .fetch_all(pool)
                .await
        })
        .map_err(db_error)?;
        rows.into_iter().map(Sandbox::try_from).collect()
    }

    async fn set_idle_timeout(&self, id: &SandboxId, timeout: Option<Duration>) -> Result<()> {
        let secs = timeout
            .map(|timeout| i64::try_from(timeout.as_secs()))
            .transpose()
            .map_err(|err| Error::Invalid(format!("an idle timeout that long: {err}")))?;
        on_pool!(self.pool, |pool| {
            sqlx::query("update sandboxes set idle_timeout_secs = $2 where id = $1")
                .bind(&id.0)
                .bind(secs)
                .execute(pool)
                .await
                .map(drop)
        })
        .map_err(db_error)
    }

    async fn set_keep_awake_until(
        &self,
        id: &SandboxId,
        until: Option<DateTime<Utc>>,
    ) -> Result<()> {
        on_pool!(self.pool, |pool| {
            sqlx::query("update sandboxes set keep_awake_until = $2 where id = $1")
                .bind(&id.0)
                .bind(until)
                .execute(pool)
                .await
                .map(drop)
        })
        .map_err(db_error)
    }

    async fn client_state(&self, id: &SandboxId, client: &str) -> Result<Option<String>> {
        on_pool!(self.pool, |pool| {
            sqlx::query_scalar(
                "select state from sandbox_client_states where sandbox_id = $1 and client = $2",
            )
            .bind(&id.0)
            .bind(client)
            .fetch_optional(pool)
            .await
        })
        .map_err(db_error)
    }

    async fn set_client_state(&self, id: &SandboxId, client: &str, state: &str) -> Result<()> {
        on_pool!(self.pool, |pool| {
            sqlx::query(
                "insert into sandbox_client_states (sandbox_id, client, state) \
                 select id, $2, $3 from sandboxes where id = $1 \
                 on conflict (sandbox_id, client) do update set state = excluded.state",
            )
            .bind(&id.0)
            .bind(client)
            .bind(state)
            .execute(pool)
            .await
            .map(drop)
        })
        .map_err(db_error)
    }
}
