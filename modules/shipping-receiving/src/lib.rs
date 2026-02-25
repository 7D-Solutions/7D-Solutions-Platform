pub mod config;
pub mod domain;
pub mod http;
pub mod outbox;
pub mod routes;

pub use config::Config;

pub struct AppState {
    pub pool: sqlx::PgPool,
}
