use async_trait::async_trait;
use corral_core::domain::{Handle, User, UserId};
use corral_core::ports::UserRepo;
use corral_core::{Error, Result};
use sqlx::Row;
use sqlx::postgres::{PgPool, PgRow};

use crate::decode::{from_text, parsed, text};
use crate::error::db_error;

pub(crate) struct Users {
    pub(crate) pool: PgPool,
}

fn from_row(row: &PgRow) -> sqlx::Result<User> {
    Ok(User {
        id: UserId(row.try_get("id")?),
        subject: row.try_get("subject")?,
        handle: parsed(row, "handle", Handle::parse)?,
        email: row.try_get("email")?,
        role: parsed(row, "role", from_text)?,
        enabled: row.try_get("enabled")?,
        created_at: row.try_get("created_at")?,
    })
}

#[async_trait]
impl UserRepo for Users {
    async fn by_subject(&self, subject: &str) -> Result<Option<User>> {
        sqlx::query(
            "select id, subject, handle, email, role, enabled, created_at \
             from users where subject = $1",
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
             from users where handle = $1",
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
             from users where id = $1",
        )
        .bind(id.0)
        .try_map(|row| from_row(&row))
        .fetch_optional(&self.pool)
        .await
        .map_err(db_error)
    }

    async fn insert(&self, user: &User) -> Result<()> {
        sqlx::query(
            "insert into users (id, subject, handle, email, role, enabled, created_at) \
             values ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(user.id.0)
        .bind(&user.subject)
        .bind(user.handle.as_str())
        .bind(&user.email)
        .bind(text(&user.role))
        .bind(user.enabled)
        .bind(user.created_at)
        .execute(&self.pool)
        .await
        .map_err(db_error)?;
        Ok(())
    }

    async fn update(&self, user: &User) -> Result<()> {
        let updated = sqlx::query(
            "update users set subject = $1, handle = $2, email = $3, role = $4, enabled = $5, \
             created_at = $6 where id = $7",
        )
        .bind(&user.subject)
        .bind(user.handle.as_str())
        .bind(&user.email)
        .bind(text(&user.role))
        .bind(user.enabled)
        .bind(user.created_at)
        .bind(user.id.0)
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
