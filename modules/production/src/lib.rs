pub mod config;
pub mod consumers;
pub mod db;
pub mod domain;
pub mod events;
pub mod http;
pub mod metrics;

pub use config::Config;
pub use domain::bom_client::BomRevisionClient;
pub use domain::numbering_client::NumberingClient;

pub use platform_client_outside_processing::OutsideProcessingClient;

#[derive(Clone)]
pub struct AppState {
    pub pool: sqlx::PgPool,
    pub metrics: std::sync::Arc<metrics::ProductionMetrics>,
    pub numbering: std::sync::Arc<NumberingClient>,
    pub bom: std::sync::Arc<BomRevisionClient>,
    pub op_client: std::sync::Arc<OutsideProcessingClient>,
}
