//! Per-handler permission extractors.
//!
//! Each struct is an axum `FromRequestParts` extractor that:
//! 1. Reads the Bearer token from the Authorization header.
//! 2. Verifies it against the service's own JwtKeys.
//! 3. Queries the DB for the caller's current effective permissions.
//! 4. Returns 401/403 if the required permission is absent.
//!
//! Using extractors rather than route layers keeps permission requirements
//! visible in the handler signature and exposes them to utoipa OpenAPI docs.

use axum::{
    extract::FromRequestParts,
    http::{header, request::Parts, StatusCode},
};
use std::sync::Arc;
use uuid::Uuid;

use super::handlers::AuthState;

/// Verified caller who holds `admin.users.read`.
///
/// Add this as a handler parameter to gate the route on that permission.
/// `user_id` and `tenant_id` are the caller's identity, available for
/// audit or further per-caller filtering inside the handler.
#[derive(Debug)]
pub struct AdminUsersRead {
    pub user_id: Uuid,
    pub tenant_id: Uuid,
}

impl FromRequestParts<Arc<AuthState>> for AdminUsersRead {
    type Rejection = (StatusCode, String);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AuthState>,
    ) -> Result<Self, Self::Rejection> {
        const PERM: &str = "admin.users.read";

        // 1. Extract Bearer token from Authorization header.
        let token = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .ok_or_else(|| {
                (
                    StatusCode::UNAUTHORIZED,
                    "missing or invalid authentication".to_string(),
                )
            })?;

        // 2. Verify JWT with the service's own keys.
        let claims = state.jwt.validate_access_token(token).map_err(|_| {
            (
                StatusCode::UNAUTHORIZED,
                "missing or invalid authentication".to_string(),
            )
        })?;

        let user_id = Uuid::parse_str(&claims.sub).map_err(|_| {
            (
                StatusCode::UNAUTHORIZED,
                "missing or invalid authentication".to_string(),
            )
        })?;
        let tenant_id = Uuid::parse_str(&claims.tenant_id).map_err(|_| {
            (
                StatusCode::UNAUTHORIZED,
                "missing or invalid authentication".to_string(),
            )
        })?;

        // 3. Service-to-service tokens bypass fine-grained permission checks.
        let is_service = claims.perms.iter().any(|p| p == "service.internal");
        if is_service {
            return Ok(AdminUsersRead { user_id, tenant_id });
        }

        // 4. Resolve current effective permissions from DB.
        //    Do NOT trust `claims.perms` — the JWT can drift behind role changes.
        let perms = crate::db::rbac::effective_permissions_for_user(&state.db, tenant_id, user_id)
            .await
            .map_err(|e| {
                tracing::error!(
                    error = %e,
                    user_id = %user_id,
                    tenant_id = %tenant_id,
                    "authz: db error resolving permissions for admin.users.read"
                );
                (StatusCode::INTERNAL_SERVER_ERROR, "db error".to_string())
            })?;

        if !perms.contains(&PERM.to_string()) {
            return Err((
                StatusCode::FORBIDDEN,
                "insufficient permissions".to_string(),
            ));
        }

        Ok(AdminUsersRead { user_id, tenant_id })
    }
}
