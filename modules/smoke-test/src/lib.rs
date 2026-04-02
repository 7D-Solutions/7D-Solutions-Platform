pub mod consumer;
pub mod http;

pub struct AppState {
    pub pool: sqlx::PgPool,
}
