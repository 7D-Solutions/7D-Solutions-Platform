/// Application state for the control-plane service

use sqlx::PgPool;

/// Shared state injected into all handlers
#[derive(Clone)]
pub struct AppState {
    /// Connection pool to the tenant-registry database
    pub pool: PgPool,
}

impl AppState {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}
