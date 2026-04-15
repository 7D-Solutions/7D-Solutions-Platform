//! Accrual HTTP routes (bd-3qa, bd-2ob)
//!
//! POST /api/gl/accruals/templates           — Create an accrual template
//! POST /api/gl/accruals/create              — Create an accrual instance from template
//! POST /api/gl/accruals/reversals/execute   — Execute auto-reversals for a target period

use axum::{extract::State, http::StatusCode, response::IntoResponse, Extension, Json};
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use std::sync::Arc;

use super::auth::with_request_id;
use crate::accruals;
use crate::AppState;
use platform_sdk::extract_tenant;

#[utoipa::path(post, path = "/api/gl/accruals/templates", tag = "Accruals",
    request_body = crate::accruals::CreateTemplateRequest,
    responses((status = 201, description = "Accrual template created", body = crate::accruals::TemplateResult)),
    security(("bearer" = [])))]
pub async fn create_template_handler(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Json(mut body): Json<accruals::CreateTemplateRequest>,
) -> impl IntoResponse {
    match extract_tenant(&claims) {
        Ok(tid) => body.tenant_id = tid,
        Err(e) => return with_request_id(e, &ctx).into_response(),
    }
    match accruals::create_template(&app_state.pool, &body).await {
        Ok(result) => (StatusCode::CREATED, Json(result)).into_response(),
        Err(e) => {
            let api_err = match &e {
                accruals::AccrualError::Validation(_) => ApiError::bad_request(e.to_string()),
                _ => ApiError::internal(e.to_string()),
            };
            with_request_id(api_err, &ctx).into_response()
        }
    }
}

#[utoipa::path(post, path = "/api/gl/accruals/create", tag = "Accruals",
    request_body = crate::accruals::CreateAccrualRequest,
    responses((status = 201, description = "Accrual instance created", body = crate::accruals::AccrualResult)),
    security(("bearer" = [])))]
pub async fn create_accrual_handler(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Json(mut body): Json<accruals::CreateAccrualRequest>,
) -> impl IntoResponse {
    match extract_tenant(&claims) {
        Ok(tid) => body.tenant_id = tid,
        Err(e) => return with_request_id(e, &ctx).into_response(),
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
            let api_err = match &e {
                accruals::AccrualError::Validation(_) => ApiError::bad_request(e.to_string()),
                _ => ApiError::internal(e.to_string()),
            };
            with_request_id(api_err, &ctx).into_response()
        }
    }
}

#[utoipa::path(post, path = "/api/gl/accruals/reversals/execute", tag = "Accruals",
    request_body = crate::accruals::ExecuteReversalsRequest,
    responses((status = 200, description = "Auto-reversals executed", body = crate::accruals::ExecuteReversalsResult)),
    security(("bearer" = [])))]
pub async fn execute_reversals_handler(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Json(mut body): Json<accruals::ExecuteReversalsRequest>,
) -> impl IntoResponse {
    match extract_tenant(&claims) {
        Ok(tid) => body.tenant_id = tid,
        Err(e) => return with_request_id(e, &ctx).into_response(),
    }
    match accruals::execute_auto_reversals(&app_state.pool, &body).await {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(e) => {
            let api_err = match &e {
                accruals::AccrualError::Validation(_) => ApiError::bad_request(e.to_string()),
                _ => ApiError::internal(e.to_string()),
            };
            with_request_id(api_err, &ctx).into_response()
        }
    }
}
