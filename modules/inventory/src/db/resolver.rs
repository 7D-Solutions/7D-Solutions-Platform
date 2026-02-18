use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::time::Duration;

/// Centralized DB pool resolver for Inventory module.
/// This is the ONLY place where PgPool instances should be created.
///
/// # Architecture
/// This resolver serves as a seam for future tenant routing and PDAA
/// (Per-Database-Per-App). The `app_id` parameter scopes the connection to
/// the correct tenant database when multi-tenant routing is activated.
/// Currently resolves directly to `database_url`.
///
/// All pool creation must go through this function to enable centralized
/// routing logic in future beads.
pub async fn resolve_pool(database_url: &str) -> Result<PgPool, sqlx::Error> {
    let max_connections = std::env::var("DB_MAX_CONNECTIONS")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(10);

    let acquire_timeout_secs = std::env::var("DB_ACQUIRE_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(10);

    PgPoolOptions::new()
        .max_connections(max_connections)
        .idle_timeout(Some(Duration::from_secs(300)))
        .max_lifetime(Some(Duration::from_secs(1800)))
        .acquire_timeout(Duration::from_secs(acquire_timeout_secs))
        .connect(database_url)
        .await
}

/// Derive inventory database URL for a given app_id.
///
/// Naming convention: `{base_url_without_dbname}/inventory_{app_id}`
/// This is the seam for per-tenant DB isolation (PDAA pattern).
pub fn db_url_for_app(base_url: &str, _app_id: &str) -> String {
    // Phase 25 Wave 0: return base_url as-is.
    // Future beads will append app_id-scoped DB name once tenant routing is live.
    base_url.to_string()
}
