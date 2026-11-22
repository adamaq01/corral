use chrono::{DateTime, Utc};
use corral_core::domain::{Handle, Role};
use corral_core::{Error, Result};
use corral_sql::{Migrations, Pool, db_error, on_pool, parsed, unique_as, variant, variant_name};

pub(crate) struct Account {
    pub username: Handle,
    pub password_hash: String,
    pub role: Role,
    pub created_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct AccountRow {
    username: String,
    password_hash: String,
    role: String,
    created_at: DateTime<Utc>,
}

impl TryFrom<AccountRow> for Account {
    type Error = Error;

    fn try_from(row: AccountRow) -> Result<Self> {
        Ok(Self {
            username: parsed("username", Handle::parse(&row.username))?,
            password_hash: row.password_hash,
            role: variant("role", row.role)?,
            created_at: row.created_at,
        })
    }
}

pub(crate) struct AccountTable {
    pool: Pool,
}

impl AccountTable {
    pub async fn open(pool: Pool) -> Result<Self> {
        pool.migrate(Migrations::new(
            "local_accounts",
            sqlx::migrate!("migrations/sqlite"),
            sqlx::migrate!("migrations/postgres"),
        ))
        .await?;
        Ok(Self { pool })
    }

    pub async fn get(&self, username: &Handle) -> Result<Option<Account>> {
        let row: Option<AccountRow> = on_pool!(self.pool, |pool| {
            sqlx::query_as(
                "select username, password_hash, role, created_at \
                 from local_accounts where username = $1",
            )
            .bind(username.as_str())
            .fetch_optional(pool)
            .await
        })
        .map_err(db_error)?;
        row.map(Account::try_from).transpose()
    }

    pub async fn insert(&self, account: &Account) -> Result<()> {
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
        .map_err(unique_as(format!(
            "account {} already exists",
            account.username
        )))
    }

    pub async fn set_password_hash(&self, username: &Handle, hash: &str) -> Result<bool> {
        let updated = on_pool!(self.pool, |pool| {
            sqlx::query("update local_accounts set password_hash = $1 where username = $2")
                .bind(hash)
                .bind(username.as_str())
                .execute(pool)
                .await
                .map(|done| done.rows_affected())
        })
        .map_err(db_error)?;
        Ok(updated > 0)
    }

    pub async fn delete(&self, username: &Handle) -> Result<bool> {
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

    pub async fn list(&self) -> Result<Vec<Account>> {
        let rows: Vec<AccountRow> = on_pool!(self.pool, |pool| {
            sqlx::query_as(
                "select username, password_hash, role, created_at \
                 from local_accounts order by username",
            )
            .fetch_all(pool)
            .await
        })
        .map_err(db_error)?;
        rows.into_iter().map(Account::try_from).collect()
    }
}
