use axum::{routing::post, Router};
use std::sync::Arc;

use crate::auth::handlers;

pub fn router(state: Arc<handlers::AuthState>) -> Router {
    Router::new()
        .route("/api/auth/register", post(handlers::register))
        .route("/api/auth/login", post(handlers::login))
        .route("/api/auth/refresh", post(handlers::refresh))
        .route("/api/auth/logout", post(handlers::logout))
        .with_state(state)
}
