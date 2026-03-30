use event_bus::EventBus;
use std::sync::Arc;

pub mod bus;
pub mod config;
pub mod consumers;
pub mod db;
pub mod domain;
pub mod events;
pub mod http;
pub mod metrics;

pub use bus::BusHealth;
pub use config::Config;

/// Application state shared across HTTP handlers
#[derive(Clone)]
pub struct AppState {
    pub pool: sqlx::PgPool,
    pub metrics: Arc<metrics::InventoryMetrics>,
    pub event_bus: Arc<dyn EventBus>,
    pub bus_health: Arc<BusHealth>,
}
