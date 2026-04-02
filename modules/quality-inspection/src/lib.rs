pub mod config;
pub mod consumers;
pub mod db;
pub mod domain;
pub mod events;
pub mod http;
pub mod metrics;

pub use config::Config;

#[derive(Clone)]
pub struct AppState {
    pub pool: sqlx::PgPool,
    pub wc_client: platform_sdk::PlatformClient,
    pub metrics: std::sync::Arc<metrics::QualityInspectionMetrics>,
}
