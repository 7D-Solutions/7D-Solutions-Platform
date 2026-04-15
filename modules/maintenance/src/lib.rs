//! Maintenance module — library crate.
//!
//! Exposes public modules so integration tests can call handlers directly.

pub mod config;
pub mod consumers;
pub mod domain;
pub mod events;
pub mod http;
pub mod metrics;
pub mod outbox;

pub use config::Config;

/// Application state shared across HTTP handlers and background tasks.
pub struct AppState {
    pub pool: sqlx::PgPool,
    pub metrics: std::sync::Arc<metrics::MaintenanceMetrics>,
}
