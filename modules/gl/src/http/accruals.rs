//! Accrual HTTP routes (bd-3qa, bd-2ob)
//!
//! POST /api/gl/accruals/templates           — Create an accrual template
//! POST /api/gl/accruals/create              — Create an accrual instance from template
//! POST /api/gl/accruals/reversals/execute   — Execute auto-reversals for a target period

use axum::{extract::State, http::StatusCode, response::IntoResponse, Extension, Json};
use security::VerifiedClaims;
use std::sync::Arc;

use super::auth::extract_tenant;
use crate::accruals;
use crate::AppState;

// ============================================================================
// Error response
// ============================================================================

#[derive(serde::Serialize)]
struct ErrorBody {
    error: String,
}

// ============================================================================
// POST /api/gl/accruals/templates
// ============================================================================

pub async fn create_template_handler(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut body): Json<accruals::CreateTemplateRequest>,
) -> impl IntoResponse {
    // Override client-supplied tenant_id with JWT claims
    match extract_tenant(&claims) {
        Ok(tid) => body.tenant_id = tid,
        Err((status, msg)) => {
            return (status, Json(ErrorBody { error: msg })).into_response();
        }
    }
    match accruals::create_template(&app_state.pool, &body).await {
        Ok(result) => (StatusCode::CREATED, Json(result)).into_response(),
        Err(e) => {
            let status = match &e {
                accruals::AccrualError::Validation(_) => StatusCode::BAD_REQUEST,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            (
                status,
                Json(ErrorBody {
                    error: e.to_string(),
                }),
            )
                .into_response()
        }
    }
}

// ============================================================================
// POST /api/gl/accruals/create
// ============================================================================

pub async fn create_accrual_handler(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut body): Json<accruals::CreateAccrualRequest>,
) -> impl IntoResponse {
    // Override client-supplied tenant_id with JWT claims
    match extract_tenant(&claims) {
        Ok(tid) => body.tenant_id = tid,
        Err((status, msg)) => {
            return (status, Json(ErrorBody { error: msg })).into_response();
        }
    }
    match accruals::create_accrual_instance(&app_state.pool, &body).await {
        Ok(result) => {
            let status = if result.idempotent_hit {
                StatusCode::OK
            } else {
                StatusCode::CREATED
            };
            (status, Json(result)).into_response()
        }
        Err(e) => {
            let status = match &e {
                accruals::AccrualError::Validation(_) => StatusCode::BAD_REQUEST,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            (
                status,
                Json(ErrorBody {
                    error: e.to_string(),
                }),
            )
                .into_response()
        }
    }
}

// ============================================================================
// POST /api/gl/accruals/reversals/execute
// ============================================================================

pub async fn execute_reversals_handler(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut body): Json<accruals::ExecuteReversalsRequest>,
) -> impl IntoResponse {
    // Override client-supplied tenant_id with JWT claims
    match extract_tenant(&claims) {
        Ok(tid) => body.tenant_id = tid,
        Err((status, msg)) => {
            return (status, Json(ErrorBody { error: msg })).into_response();
        }
    }
    match accruals::execute_auto_reversals(&app_state.pool, &body).await {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(e) => {
            let status = match &e {
                accruals::AccrualError::Validation(_) => StatusCode::BAD_REQUEST,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            (
                status,
                Json(ErrorBody {
                    error: e.to_string(),
                }),
            )
                .into_response()
        }
    }
}
