use axum::{
    routing::{delete, get, post},
    Router,
};
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
        .route("/api/auth/sessions", get(session::list_sessions))
        .route(
            "/api/auth/sessions/{session_id}/revoke",
            post(session::revoke_session),
        )
        .route(
            "/api/auth/access-review",
            post(handlers::record_access_review),
        )
        .route(
            "/api/auth/lifecycle/{tenant_id}/{user_id}",
            get(handlers::get_user_lifecycle_timeline),
        )
        .route("/api/auth/sod/policies", post(handlers::upsert_sod_policy))
        .route("/api/auth/sod/evaluate", post(handlers::evaluate_sod))
        .route(
            "/api/auth/sod/policies/{tenant_id}/by-action/{action_key}",
            get(handlers::list_sod_policies),
        )
        .route(
            "/api/auth/sod/policies/{tenant_id}/{rule_id}",
            delete(handlers::delete_sod_policy),
        )
        .route("/api/auth/users", get(handlers::get_user_by_email))
        .route(
            "/api/auth/forgot-password",
            post(handlers_password_reset::forgot_password),
        )
        .route(
            "/api/auth/reset-password",
            post(handlers_password_reset::reset_password),
        )
        .route("/api/auth/roles", get(handlers::list_roles))
        .route("/api/auth/permissions", get(handlers::list_permissions))
        .route("/api/auth/admin/users", get(handlers::list_users_admin))
        .with_state(state)
}
