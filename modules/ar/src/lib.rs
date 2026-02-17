pub mod config;
pub mod consumer_tasks;
pub mod db;
pub mod envelope_validation;
pub mod events;
pub mod finalization;
pub mod idempotency;
pub mod idempotency_keys;
pub mod invariants;
pub mod lifecycle;
pub mod metrics;
pub mod models;
pub mod retry;
pub mod routes;
pub mod tilled;

// Re-export config types for testing
pub use config::Config;

/// AR application state shared across HTTP handlers
pub struct AppState {
    pub pool: sqlx::PgPool,
    pub metrics: std::sync::Arc<metrics::ArMetrics>,
}
