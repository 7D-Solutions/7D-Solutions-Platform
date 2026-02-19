pub mod config;
pub mod db;
pub mod domain;
pub mod events;
pub mod http;
pub mod metrics;
pub mod ops;
pub mod outbox;

pub use config::Config;

/// Party application state shared across HTTP handlers
pub struct AppState {
    pub pool: sqlx::PgPool,
    pub metrics: std::sync::Arc<metrics::PartyMetrics>,
}
