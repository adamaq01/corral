use std::sync::Arc;

use anyhow::{Context, Result};
use corral_store_sql::Store;

use crate::env;

const DEFAULT_DATABASE_URL: &str = "sqlite:///var/lib/corral/corral.db";

pub async fn connect() -> Result<Arc<Store>> {
    let url = env::var("DATABASE_URL").unwrap_or_else(|| DEFAULT_DATABASE_URL.to_owned());
    let store = Store::connect(&url)
        .await
        .context("opening the database at DATABASE_URL")?;
    Ok(Arc::new(store))
}
