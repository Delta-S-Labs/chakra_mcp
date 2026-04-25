use std::time::Duration;

use anyhow::Result;
use sqlx::postgres::{PgPool, PgPoolOptions};

/// Connect to Postgres with sane defaults. Both services call this.
pub async fn connect(database_url: &str) -> Result<PgPool> {
    let pool = PgPoolOptions::new()
        .max_connections(20)
        .min_connections(1)
        .acquire_timeout(Duration::from_secs(5))
        .idle_timeout(Some(Duration::from_secs(300)))
        .connect(database_url)
        .await?;
    Ok(pool)
}
