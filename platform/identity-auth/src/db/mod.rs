#[allow(dead_code)] // RBAC functions prepared for upcoming admin endpoints
pub mod rbac;
pub mod sod;
pub mod user_lifecycle_audit;

use sqlx::{postgres::PgPoolOptions, PgPool};
use std::time::Duration;

pub async fn create_pool(database_url: &str) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(Duration::from_secs(30))
        .after_connect(|conn, _meta| {
            Box::pin(async move {
                sqlx::query("SET statement_timeout = '10s'")
                    .execute(conn)
                    .await?;
                Ok(())
            })
        })
        .connect(database_url)
        .await
}

pub async fn run_migrations(pool: &PgPool) -> Result<(), Box<dyn std::error::Error>> {
    sqlx::migrate!("./db/migrations").run(pool).await?;
    Ok(())
}
