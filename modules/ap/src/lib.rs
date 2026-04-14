pub mod config;
pub mod consumers;
pub mod db;
pub mod domain;
pub mod events;
pub mod http;
pub mod integrations;
pub mod metrics;
pub mod outbox;

pub use config::Config;

/// AP application state shared across HTTP handlers
pub struct AppState {
    pub pool: sqlx::PgPool,
    pub metrics: std::sync::Arc<metrics::ApMetrics>,
    /// Optional read-only connection to the GL database for period pre-validation.
    /// When set, `POST /api/ap/bills` checks that the invoice date falls in an open
    /// GL period before accepting the entry (fail-fast, before any AP DB writes).
    /// If absent, AP operates without period pre-check (GL enforces on posting event).
    pub gl_pool: Option<sqlx::PgPool>,
}
