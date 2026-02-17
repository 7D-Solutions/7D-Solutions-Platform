pub mod aging;
pub mod config;
pub mod consumer_tasks;
pub mod credit_notes;
pub mod db;
pub mod dunning;
pub mod dunning_scheduler;
pub mod envelope_validation;
pub mod events;
pub mod finalization;
pub mod idempotency;
pub mod idempotency_keys;
pub mod invariants;
pub mod lifecycle;
pub mod metrics;
pub mod middleware;
pub mod models;
pub mod retry;
pub mod routes;
pub mod tax;
pub mod tilled;
pub mod usage_billing;
pub mod write_offs;

// Re-export config types for testing
pub use config::Config;

/// AR application state shared across HTTP handlers
pub struct AppState {
    pub pool: sqlx::PgPool,
    pub metrics: std::sync::Arc<metrics::ArMetrics>,
}
