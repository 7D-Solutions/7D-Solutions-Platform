pub mod config;
pub mod db;
pub mod domain;
pub mod events;
pub mod http;
pub mod metrics;

pub use config::Config;
pub use domain::inventory_client::InventoryClient;
pub use domain::numbering_client::NumberingClient;

pub struct AppState {
    pub pool: sqlx::PgPool,
    pub metrics: std::sync::Arc<metrics::BomMetrics>,
    pub numbering: NumberingClient,
    pub inventory: InventoryClient,
}
