mod keys;
mod local_accounts;
mod sandboxes;
mod users;

use corral_core::Result;
use corral_sql::{Migrations, Pool};

pub struct Store {
    pool: Pool,
}

impl Store {
    pub async fn open(pool: Pool) -> Result<Self> {
        pool.migrate(Migrations::new(
            "store",
            sqlx::migrate!("migrations/sqlite"),
            sqlx::migrate!("migrations/postgres"),
        ))
        .await?;
        Ok(Self { pool })
    }
}
