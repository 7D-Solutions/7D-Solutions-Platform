//! Billing rates + billing runs HTTP handlers.

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

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
// Query params
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct ListRatesQuery {
    pub app_id: String,
}

// ============================================================================
// Handlers
// ============================================================================

/// POST /api/timekeeping/rates
pub async fn create_rate(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateBillingRateRequest>,
) -> impl IntoResponse {
    match service::create_billing_rate(&state.pool, &req).await {
        Ok(rate) => (StatusCode::CREATED, Json(json!(rate))).into_response(),
        Err(err) => billing_error_response(err).into_response(),
    }
}

/// GET /api/timekeeping/rates
pub async fn list_rates(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ListRatesQuery>,
) -> impl IntoResponse {
    match service::list_billing_rates(&state.pool, &q.app_id).await {
        Ok(rates) => (StatusCode::OK, Json(json!(rates))).into_response(),
        Err(err) => billing_error_response(err).into_response(),
    }
}

/// POST /api/timekeeping/billing-runs
pub async fn create_billing_run(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateBillingRunRequest>,
) -> impl IntoResponse {
    match service::create_billing_run(&state.pool, &req).await {
        Ok(result) => (StatusCode::CREATED, Json(json!(result))).into_response(),
        Err(err) => billing_error_response(err).into_response(),
    }
}
