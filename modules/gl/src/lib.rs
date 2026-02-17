pub mod config;
pub mod consumer;
pub mod contracts;
pub mod db;
pub mod dlq;
pub mod domain;
pub mod events;
pub mod health;
pub mod invariants;
pub mod metrics;
pub mod repos;
pub mod revrec;
pub mod routes;
pub mod services;
pub mod validation;

// Re-export config types for testing
pub use config::Config;

pub use consumer::gl_posting_consumer::start_gl_posting_consumer;
pub use consumer::gl_reversal_consumer::start_gl_reversal_consumer;

/// Application state shared across HTTP handlers
#[derive(Clone)]
pub struct AppState {
    pub pool: sqlx::PgPool,
    pub dlq_validation_enabled: bool,
    pub metrics: std::sync::Arc<metrics::GlMetrics>,
}
