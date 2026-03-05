pub mod config;
pub mod db;
pub mod domain;
pub mod events;
pub mod http;
pub mod metrics;

pub use config::Config;

#[derive(Clone)]
pub struct AppState {
    pub pool: sqlx::PgPool,
    pub metrics: std::sync::Arc<metrics::BomMetrics>,
}
