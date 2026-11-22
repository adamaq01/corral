use async_trait::async_trait;
use chrono::{DateTime, Utc};
use corral_core::domain::{Handle, User, UserId};
use corral_core::ports::UserRepo;
use corral_core::{Error, Result};
use uuid::Uuid;

use crate::Store;
use corral_sql::{db_error, on_pool, parsed, unique_as, variant, variant_name};

#[derive(sqlx::FromRow)]
struct UserRow {
    id: Uuid,
    subject: String,
    handle: String,
    email: Option<String>,
    role: String,
    status: String,
    created_at: DateTime<Utc>,
}

impl TryFrom<UserRow> for User {
    type Error = Error;

    fn try_from(row: UserRow) -> Result<Self> {
        Ok(Self {
            id: UserId(row.id),
            subject: row.subject,
            handle: parsed("handle", Handle::parse(&row.handle))?,
            email: row.email,
            role: variant("role", row.role)?,
            status: variant("status", row.status)?,
            created_at: row.created_at,
        })
    }
}

#[async_trait]
impl UserRepo for Store {
    async fn by_subject(&self, subject: &str) -> Result<Option<User>> {
        let row: Option<UserRow> = on_pool!(self.pool, |pool| {
            sqlx::query_as(
                "select id, subject, handle, email, role, status, created_at \
                 from users where subject = $1",
            )
            .bind(subject)
            .fetch_optional(pool)
            .await
        })
        .map_err(db_error)?;
        row.map(User::try_from).transpose()
    }

    async fn by_handle(&self, handle: &Handle) -> Result<Option<User>> {
        let row: Option<UserRow> = on_pool!(self.pool, |pool| {
            sqlx::query_as(
                "select id, subject, handle, email, role, status, created_at \
                 from users where handle = $1",
            )
            .bind(handle.as_str())
            .fetch_optional(pool)
            .await
        })
        .map_err(db_error)?;
        row.map(User::try_from).transpose()
    }

    async fn get(&self, id: UserId) -> Result<Option<User>> {
        let row: Option<UserRow> = on_pool!(self.pool, |pool| {
            sqlx::query_as(
                "select id, subject, handle, email, role, status, created_at \
                 from users where id = $1",
            )
            .bind(id.0)
            .fetch_optional(pool)
            .await
        })
        .map_err(db_error)?;
        row.map(User::try_from).transpose()
    }

    async fn insert(&self, user: &User) -> Result<()> {
        on_pool!(self.pool, |pool| {
            sqlx::query(
                "insert into users (id, subject, handle, email, role, status, created_at) \
                 values ($1, $2, $3, $4, $5, $6, $7)",
            )
            .bind(user.id.0)
            .bind(&user.subject)
            .bind(user.handle.as_str())
            .bind(&user.email)
            .bind(variant_name(&user.role))
            .bind(variant_name(&user.status))
            .bind(user.created_at)
            .execute(pool)
            .await
            .map(|_| ())
        })
        .map_err(unique_as(format!("user {} already exists", user.handle)))
    }

    async fn update(&self, user: &User) -> Result<()> {
        let updated = on_pool!(self.pool, |pool| {
            sqlx::query(
                "update users set subject = $1, handle = $2, email = $3, role = $4, \
                 status = $5, created_at = $6 where id = $7",
            )
            .bind(&user.subject)
            .bind(user.handle.as_str())
            .bind(&user.email)
            .bind(variant_name(&user.role))
            .bind(variant_name(&user.status))
            .bind(user.created_at)
            .bind(user.id.0)
            .execute(pool)
            .await
            .map(|done| done.rows_affected())
        })
        .map_err(unique_as(format!("user {} already exists", user.handle)))?;
        if updated == 0 {
            return Err(Error::NotFound(format!("user {}", user.handle)));
        }
        Ok(())
    }

    async fn list(&self) -> Result<Vec<User>> {
        let rows: Vec<UserRow> = on_pool!(self.pool, |pool| {
            sqlx::query_as(
                "select id, subject, handle, email, role, status, created_at \
                 from users order by created_at, id",
            )
            .fetch_all(pool)
            .await
        })
        .map_err(db_error)?;
        rows.into_iter().map(User::try_from).collect()
    }

    async fn delete(&self, id: UserId) -> Result<bool> {
        let deleted = on_pool!(self.pool, |pool| {
            sqlx::query("delete from users where id = $1")
                .bind(id.0)
                .execute(pool)
                .await
                .map(|done| done.rows_affected())
        })
        .map_err(db_error)?;
        Ok(deleted > 0)
    }
}
