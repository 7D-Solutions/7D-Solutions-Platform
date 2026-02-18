pub mod config;
pub mod db;
pub mod events;
pub mod http;
pub mod metrics;
pub mod outbox;

pub use config::Config;

/// AP application state shared across HTTP handlers
pub struct AppState {
    pub pool: sqlx::PgPool,
    pub metrics: std::sync::Arc<metrics::ApMetrics>,
}
