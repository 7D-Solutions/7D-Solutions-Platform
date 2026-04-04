//! Numbering module — library crate.
//!
//! Tenant-scoped, idempotent, atomic sequence allocation service.

pub mod config;
pub mod db;
pub mod format;
pub mod http;
pub mod metrics;
pub mod outbox;
pub mod policy;

pub use config::Config;

/// Application state shared across HTTP handlers and background tasks.
pub struct AppState {
    pub pool: sqlx::PgPool,
    pub metrics: std::sync::Arc<metrics::NumberingMetrics>,
}
