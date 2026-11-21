use async_trait::async_trait;
use corral_core::domain::{Handle, Role, User, UserId};
use corral_core::ports::UserRepo;
use corral_core::{Error, Result};
use sqlx::Row;
use sqlx::sqlite::{SqlitePool, SqliteRow};

use crate::decode::{parsed, uuid};
use crate::error::db_error;

pub(crate) struct Users {
    pub(crate) pool: SqlitePool,
}

fn from_row(row: &SqliteRow) -> sqlx::Result<User> {
    Ok(User {
        id: UserId(uuid(row, "id")?),
        subject: row.try_get("subject")?,
        handle: parsed(row, "handle", Handle::parse)?,
        email: row.try_get("email")?,
        role: parsed(row, "role", Role::parse)?,
        enabled: row.try_get("enabled")?,
        created_at: row.try_get("created_at")?,
    })
}

#[async_trait]
impl UserRepo for Users {
    async fn by_subject(&self, subject: &str) -> Result<Option<User>> {
        sqlx::query(
            "select id, subject, handle, email, role, enabled, created_at \
             from users where subject = ?",
        )
        .bind(subject)
        .try_map(|row| from_row(&row))
        .fetch_optional(&self.pool)
        .await
        .map_err(db_error)
    }

    async fn by_handle(&self, handle: &Handle) -> Result<Option<User>> {
        sqlx::query(
            "select id, subject, handle, email, role, enabled, created_at \
             from users where handle = ?",
        )
        .bind(handle.as_str())
        .try_map(|row| from_row(&row))
        .fetch_optional(&self.pool)
        .await
        .map_err(db_error)
    }

    async fn get(&self, id: UserId) -> Result<Option<User>> {
        sqlx::query(
            "select id, subject, handle, email, role, enabled, created_at \
             from users where id = ?",
        )
        .bind(id.0.hyphenated())
        .try_map(|row| from_row(&row))
        .fetch_optional(&self.pool)
        .await
        .map_err(db_error)
    }

    async fn insert(&self, user: &User) -> Result<()> {
        sqlx::query(
            "insert into users (id, subject, handle, email, role, enabled, created_at) \
             values (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(user.id.0.hyphenated())
        .bind(&user.subject)
        .bind(user.handle.as_str())
        .bind(&user.email)
        .bind(user.role.as_str())
        .bind(user.enabled)
        .bind(user.created_at)
        .execute(&self.pool)
        .await
        .map_err(db_error)?;
        Ok(())
    }

    async fn update(&self, user: &User) -> Result<()> {
        let updated = sqlx::query(
            "update users set subject = ?, handle = ?, email = ?, role = ?, enabled = ?, \
             created_at = ? where id = ?",
        )
        .bind(&user.subject)
        .bind(user.handle.as_str())
        .bind(&user.email)
        .bind(user.role.as_str())
        .bind(user.enabled)
        .bind(user.created_at)
        .bind(user.id.0.hyphenated())
        .execute(&self.pool)
        .await
        .map_err(db_error)?;
        if updated.rows_affected() == 0 {
            return Err(Error::NotFound(format!("user {}", user.handle)));
        }
        Ok(())
    }

    async fn list(&self) -> Result<Vec<User>> {
        sqlx::query(
            "select id, subject, handle, email, role, enabled, created_at \
             from users order by created_at, id",
        )
        .try_map(|row| from_row(&row))
        .fetch_all(&self.pool)
        .await
        .map_err(db_error)
    }
}
