pub mod config;
pub mod consumers;
pub mod db;
pub mod domain;
pub mod events;
pub mod http;
pub mod integrations;
pub mod metrics;
pub mod outbox;
pub mod routes;

pub use config::Config;
pub use integrations::inventory_client::InventoryIntegration;
pub use integrations::quality_gate_client::QualityGateIntegration;

pub struct AppState {
    pub pool: sqlx::PgPool,
    pub metrics: std::sync::Arc<metrics::ShippingReceivingMetrics>,
    pub inventory: InventoryIntegration,
    pub quality_gate: QualityGateIntegration,
}
