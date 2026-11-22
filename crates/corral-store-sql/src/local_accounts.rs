use async_trait::async_trait;
use chrono::{DateTime, Utc};
use corral_core::domain::{Handle, LocalAccount};
use corral_core::ports::LocalAccountRepo;
use corral_core::{Error, Result};

use crate::Store;
use corral_sql::{db_error, on_pool, parsed, variant, variant_name};

#[derive(sqlx::FromRow)]
struct LocalAccountRow {
    username: String,
    password_hash: String,
    role: String,
    created_at: DateTime<Utc>,
}

impl TryFrom<LocalAccountRow> for LocalAccount {
    type Error = Error;

    fn try_from(row: LocalAccountRow) -> Result<Self> {
        Ok(Self {
            username: parsed("username", Handle::parse(&row.username))?,
            password_hash: row.password_hash,
            role: variant("role", row.role)?,
            created_at: row.created_at,
        })
    }
}

#[async_trait]
impl LocalAccountRepo for Store {
    async fn get(&self, username: &Handle) -> Result<Option<LocalAccount>> {
        let row: Option<LocalAccountRow> = on_pool!(self.pool, |pool| {
            sqlx::query_as(
                "select username, password_hash, role, created_at \
                 from local_accounts where username = $1",
            )
            .bind(username.as_str())
            .fetch_optional(pool)
            .await
        })
        .map_err(db_error)?;
        row.map(LocalAccount::try_from).transpose()
    }

    async fn insert(&self, account: &LocalAccount) -> Result<()> {
        on_pool!(self.pool, |pool| {
            sqlx::query(
                "insert into local_accounts (username, password_hash, role, created_at) \
                 values ($1, $2, $3, $4)",
            )
            .bind(account.username.as_str())
            .bind(&account.password_hash)
            .bind(variant_name(&account.role))
            .bind(account.created_at)
            .execute(pool)
            .await
            .map(|_| ())
        })
        .map_err(db_error)
    }

    async fn update(&self, account: &LocalAccount) -> Result<()> {
        let updated = on_pool!(self.pool, |pool| {
            sqlx::query(
                "update local_accounts set password_hash = $1, role = $2, created_at = $3 \
                 where username = $4",
            )
            .bind(&account.password_hash)
            .bind(variant_name(&account.role))
            .bind(account.created_at)
            .bind(account.username.as_str())
            .execute(pool)
            .await
            .map(|done| done.rows_affected())
        })
        .map_err(db_error)?;
        if updated == 0 {
            return Err(Error::NotFound(format!(
                "local account {}",
                account.username
            )));
        }
        Ok(())
    }

    async fn delete(&self, username: &Handle) -> Result<bool> {
        let deleted = on_pool!(self.pool, |pool| {
            sqlx::query("delete from local_accounts where username = $1")
                .bind(username.as_str())
                .execute(pool)
                .await
                .map(|done| done.rows_affected())
        })
        .map_err(db_error)?;
        Ok(deleted > 0)
    }

    async fn list(&self) -> Result<Vec<LocalAccount>> {
        let rows: Vec<LocalAccountRow> = on_pool!(self.pool, |pool| {
            sqlx::query_as(
                "select username, password_hash, role, created_at \
                 from local_accounts order by username",
            )
            .fetch_all(pool)
            .await
        })
        .map_err(db_error)?;
        rows.into_iter().map(LocalAccount::try_from).collect()
    }
}
