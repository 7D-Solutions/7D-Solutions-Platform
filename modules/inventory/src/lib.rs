pub mod config;
pub mod db;
pub mod events;
pub mod metrics;
pub mod routes;

pub use config::Config;

/// Application state shared across HTTP handlers
#[derive(Clone)]
pub struct AppState {
    pub pool: sqlx::PgPool,
    pub metrics: std::sync::Arc<metrics::InventoryMetrics>,
}
