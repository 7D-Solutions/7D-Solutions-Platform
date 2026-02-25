//! Billing rates + billing runs HTTP handlers.

use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use security::VerifiedClaims;
use serde_json::json;
use std::sync::Arc;

use super::tenant::extract_tenant;
use crate::{
    domain::billing::{
        models::{BillingError, CreateBillingRateRequest, CreateBillingRunRequest},
        service,
    },
    AppState,
};

// ============================================================================
// Error mapping
// ============================================================================

fn billing_error_response(err: BillingError) -> impl IntoResponse {
    match err {
        BillingError::Validation(msg) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "validation_error", "message": msg })),
        ),
        BillingError::NoBillableEntries => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "no_billable_entries",
                "message": "No unbilled billable entries found for the specified period"
            })),
        ),
        BillingError::Database(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "database_error", "message": e.to_string() })),
        ),
    }
}

// ============================================================================
// Handlers
// ============================================================================

/// POST /api/timekeeping/rates
pub async fn create_rate(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<CreateBillingRateRequest>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    req.app_id = app_id;
    match service::create_billing_rate(&state.pool, &req).await {
        Ok(rate) => (StatusCode::CREATED, Json(json!(rate))).into_response(),
        Err(err) => billing_error_response(err).into_response(),
    }
}

/// GET /api/timekeeping/rates
pub async fn list_rates(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match service::list_billing_rates(&state.pool, &app_id).await {
        Ok(rates) => (StatusCode::OK, Json(json!(rates))).into_response(),
        Err(err) => billing_error_response(err).into_response(),
    }
}

/// POST /api/timekeeping/billing-runs
pub async fn create_billing_run(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<CreateBillingRunRequest>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    req.app_id = app_id;
    match service::create_billing_run(&state.pool, &req).await {
        Ok(result) => (StatusCode::CREATED, Json(json!(result))).into_response(),
        Err(err) => billing_error_response(err).into_response(),
    }
}
