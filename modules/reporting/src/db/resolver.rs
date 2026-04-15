use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

/// Centralized DB pool resolver for Reporting module.
///
/// This is the ONLY place where PgPool instances are created.
/// DB name follows the reporting_{app_id}_db convention; the caller supplies the
/// fully-resolved DATABASE_URL (e.g. postgres://user:pass@host/reporting_default_db).
///
/// # Architecture seam
/// This resolver is the injection point for per-app-id tenant routing.
/// Reporting is READ-ONLY — no cross-module DB writes permitted.
pub async fn resolve_pool(database_url: &str) -> Result<PgPool, sqlx::Error> {
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
