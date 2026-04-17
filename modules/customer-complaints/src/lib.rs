pub mod config;
pub mod consumers;
pub mod domain;
pub mod events;
pub mod http;
pub mod metrics;

pub use config::Config;

pub struct AppState {
    pub pool: sqlx::PgPool,
    pub metrics: std::sync::Arc<metrics::CcMetrics>,
}
