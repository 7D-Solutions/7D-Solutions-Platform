pub mod config;
pub mod db;
pub mod domain;
pub mod http;
pub mod ingest;
pub mod metrics;

pub use config::Config;

/// Reporting application state shared across HTTP handlers
pub struct AppState {
    pub pool: sqlx::PgPool,
    pub metrics: std::sync::Arc<metrics::ReportingMetrics>,
}
