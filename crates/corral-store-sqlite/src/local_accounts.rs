use async_trait::async_trait;
use corral_core::domain::{Handle, LocalAccount};
use corral_core::ports::LocalAccountRepo;
use corral_core::{Error, Result};
use sqlx::Row;
use sqlx::sqlite::{SqlitePool, SqliteRow};

use crate::decode::{from_text, parsed, text};
use crate::error::db_error;

pub(crate) struct LocalAccounts {
    pub(crate) pool: SqlitePool,
}

fn from_row(row: &SqliteRow) -> sqlx::Result<LocalAccount> {
    Ok(LocalAccount {
        username: parsed(row, "username", Handle::parse)?,
        password_hash: row.try_get("password_hash")?,
        role: parsed(row, "role", from_text)?,
        created_at: row.try_get("created_at")?,
    })
}

#[async_trait]
impl LocalAccountRepo for LocalAccounts {
    async fn get(&self, username: &Handle) -> Result<Option<LocalAccount>> {
        sqlx::query(
            "select username, password_hash, role, created_at \
             from local_accounts where username = ?",
        )
        .bind(username.as_str())
        .try_map(|row| from_row(&row))
        .fetch_optional(&self.pool)
        .await
        .map_err(db_error)
    }

    async fn insert(&self, account: &LocalAccount) -> Result<()> {
        sqlx::query(
            "insert into local_accounts (username, password_hash, role, created_at) \
             values (?, ?, ?, ?)",
        )
        .bind(account.username.as_str())
        .bind(&account.password_hash)
        .bind(text(&account.role))
        .bind(account.created_at)
        .execute(&self.pool)
        .await
        .map_err(db_error)?;
        Ok(())
    }

    async fn update(&self, account: &LocalAccount) -> Result<()> {
        let updated = sqlx::query(
            "update local_accounts set password_hash = ?, role = ?, created_at = ? \
             where username = ?",
        )
        .bind(&account.password_hash)
        .bind(text(&account.role))
        .bind(account.created_at)
        .bind(account.username.as_str())
        .execute(&self.pool)
        .await
        .map_err(db_error)?;
        if updated.rows_affected() == 0 {
            return Err(Error::NotFound(format!(
                "local account {}",
                account.username
            )));
        }
        Ok(())
    }

    async fn delete(&self, username: &Handle) -> Result<bool> {
        let deleted = sqlx::query("delete from local_accounts where username = ?")
            .bind(username.as_str())
            .execute(&self.pool)
            .await
            .map_err(db_error)?;
        Ok(deleted.rows_affected() > 0)
    }

    async fn list(&self) -> Result<Vec<LocalAccount>> {
        sqlx::query(
            "select username, password_hash, role, created_at \
             from local_accounts order by username",
        )
        .try_map(|row| from_row(&row))
        .fetch_all(&self.pool)
        .await
        .map_err(db_error)
    }
}
