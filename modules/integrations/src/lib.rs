pub mod config;
pub mod db;
pub mod http;
pub mod metrics;
pub mod ops;

pub use config::Config;

/// Integrations application state shared across HTTP handlers
pub struct AppState {
    pub pool: sqlx::PgPool,
    pub metrics: std::sync::Arc<metrics::IntegrationsMetrics>,
}
