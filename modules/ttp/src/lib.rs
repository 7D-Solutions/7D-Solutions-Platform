pub mod clients;
pub mod config;
pub mod db;
pub mod domain;
pub mod events;
pub mod http;
pub mod metrics;
pub mod ops;

pub use config::Config;

/// TTP application state shared across HTTP handlers
pub struct AppState {
    pub pool: sqlx::PgPool,
    pub metrics: std::sync::Arc<metrics::TtpMetrics>,
    pub registry_client: clients::tenant_registry::TenantRegistryClient,
    pub ar_client: clients::ar::ArClient,
}
