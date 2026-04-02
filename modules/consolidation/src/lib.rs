pub mod config;
pub mod db;
pub mod domain;
pub mod http;
pub mod integrations;
pub mod metrics;
pub mod ops;

pub use config::Config;

use integrations::gl::client::GlClient;

/// Consolidation application state shared across HTTP handlers
pub struct AppState {
    pub pool: sqlx::PgPool,
    pub metrics: std::sync::Arc<metrics::ConsolidationMetrics>,
    pub gl_client: GlClient,
}

impl AppState {
    /// Build a GL HTTP client from the stored platform client.
    pub fn gl_client(&self) -> GlClient {
        self.gl_client.clone()
    }
}
