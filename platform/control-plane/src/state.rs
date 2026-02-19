/// Application state for the control-plane service

use sqlx::PgPool;

/// Shared state injected into all handlers
#[derive(Clone)]
pub struct AppState {
    /// Connection pool to the tenant-registry database
    pub pool: PgPool,
    /// Connection pool to the AR database (for platform billing runs).
    /// None if AR_DATABASE_URL is not configured or unreachable at startup.
    pub ar_pool: Option<PgPool>,
}

impl AppState {
    pub fn new(pool: PgPool, ar_pool: Option<PgPool>) -> Self {
        Self { pool, ar_pool }
    }
}
