#[allow(dead_code)] // RBAC functions prepared for upcoming admin endpoints
pub mod rbac;
pub mod sod;
pub mod user_lifecycle_audit;

use sqlx::{postgres::PgPoolOptions, PgPool};

pub async fn create_pool(database_url: &str) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(10)
        .connect(database_url)
        .await
}

pub async fn run_migrations(pool: &PgPool) -> Result<(), Box<dyn std::error::Error>> {
    sqlx::migrate!("./db/migrations").run(pool).await?;
    Ok(())
}
