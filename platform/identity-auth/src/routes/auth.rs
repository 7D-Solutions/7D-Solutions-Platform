use axum::{routing::post, Router};
use std::sync::Arc;

use crate::auth::handlers;
use crate::auth::handlers_password_reset;
use crate::auth::session;

pub fn router(state: Arc<handlers::AuthState>) -> Router {
    Router::new()
        .route("/api/auth/register", post(handlers::register))
        .route("/api/auth/login", post(handlers::login))
        .route("/api/auth/refresh", post(session::refresh))
        .route("/api/auth/logout", post(session::logout))
        .route("/api/auth/forgot-password", post(handlers_password_reset::forgot_password))
        .with_state(state)
}
