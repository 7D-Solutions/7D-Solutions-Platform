use sqlx::postgres::{PgPool, PgPoolOptions};
use std::time::Duration;

/// Initialize a connection pool to the PostgreSQL database
pub async fn init_pool(database_url: &str) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(Duration::from_secs(3))
        .connect(database_url)
        .await
}
