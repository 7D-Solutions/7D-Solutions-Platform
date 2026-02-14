use sqlx::postgres::{PgPool, PgPoolOptions};
use std::time::Duration;

/// Initialize a connection pool to the PostgreSQL database
///
/// # Connection Limits
/// - Production default: 10 connections
/// - Test mode: Respects `DB_MAX_CONNECTIONS` env var (recommended: 1-2)
/// - This prevents E2E test resource exhaustion (316+ tests × 10 connections = infrastructure collapse)
pub async fn init_pool(database_url: &str) -> Result<PgPool, sqlx::Error> {
    // Allow test mode to cap connections via env var to prevent OOM kills
    let max_connections = std::env::var("DB_MAX_CONNECTIONS")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(10); // Production default

    let min_connections = std::env::var("DB_MIN_CONNECTIONS")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(0);

    // Configurable timeout for different environments
    // Tests may need longer timeout (10s) for nested service calls
    // Production typically uses 3s for fast-fail behavior
    let acquire_timeout_secs = std::env::var("DB_ACQUIRE_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(3);

    PgPoolOptions::new()
        .max_connections(max_connections)
        .min_connections(min_connections)
        .acquire_timeout(Duration::from_secs(acquire_timeout_secs))
        .connect(database_url)
        .await
}
