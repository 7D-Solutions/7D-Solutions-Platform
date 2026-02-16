use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

/// Centralized DB pool resolver for Payments module.
/// This is the ONLY place where PgPool instances should be created.
///
/// # Phase 16 Architecture
/// This resolver serves as a seam for future tenant routing and PDAA (Per-Database-Per-App).
/// All pool creation must go through this function to enable centralized routing logic.
pub async fn resolve_pool(database_url: &str) -> Result<PgPool, sqlx::Error> {
    // Configure connection pool based on environment
    let is_test = std::env::var("NODE_ENV").unwrap_or_default() == "test";
    let max_connections = if is_test { 5 } else { 10 };
    let idle_timeout = if is_test {
        std::time::Duration::from_secs(60)
    } else {
        std::time::Duration::from_secs(300)
    };

    PgPoolOptions::new()
        .max_connections(max_connections)
        .idle_timeout(Some(idle_timeout))
        .max_lifetime(Some(std::time::Duration::from_secs(1800)))
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(database_url)
        .await
}
