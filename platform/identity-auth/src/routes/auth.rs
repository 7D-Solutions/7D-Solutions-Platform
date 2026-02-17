use axum::{routing::post, Router};
use std::sync::Arc;

use crate::auth::handlers;
use crate::auth::session;

pub fn router(state: Arc<handlers::AuthState>) -> Router {
    Router::new()
        .route("/api/auth/register", post(handlers::register))
        .route("/api/auth/login", post(handlers::login))
        .route("/api/auth/refresh", post(session::refresh))
        .route("/api/auth/logout", post(session::logout))
        .with_state(state)
}
