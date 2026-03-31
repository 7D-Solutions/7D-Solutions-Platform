pub mod accruals;
pub mod accruals_reversal;
pub mod config;
pub mod consumers;
pub mod contracts;
pub mod db;
pub mod dlq;
pub mod domain;
pub mod events;
pub mod exports;
pub mod health;
pub mod http;
pub mod invariants;
pub mod metrics;
pub mod repos;
pub mod revrec;
pub mod services;
pub mod validation;

// Re-export config types for testing
pub use config::Config;

pub use consumers::gl_posting_consumer::start_gl_posting_consumer;
pub use consumers::gl_reversal_consumer::start_gl_reversal_consumer;

/// Application state shared across HTTP handlers
#[derive(Clone)]
pub struct AppState {
    pub pool: sqlx::PgPool,
    pub dlq_validation_enabled: bool,
    pub metrics: std::sync::Arc<metrics::GlMetrics>,
}
