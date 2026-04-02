//! Billing rates + billing runs HTTP handlers.

use axum::{extract::State, http::StatusCode, Extension, Json};
use platform_http_contracts::{ApiError, PaginatedResponse};
use security::VerifiedClaims;
use std::sync::Arc;

use platform_sdk::extract_tenant;
use crate::{
    domain::billing::{
        models::{
            BillingError, BillingRate, BillingRunResult, CreateBillingRateRequest,
            CreateBillingRunRequest,
        },
        service,
    },
    AppState,
};

// ============================================================================
// Error mapping
// ============================================================================

fn map_billing_error(err: BillingError) -> ApiError {
    match err {
        BillingError::Validation(msg) => ApiError::new(422, "validation_error", msg),
        BillingError::NoBillableEntries => ApiError::new(
            422,
            "no_billable_entries",
            "No unbilled billable entries found for the specified period",
        ),
        BillingError::Database(e) => ApiError::internal(e.to_string()),
    }
}

// ============================================================================
// Handlers
// ============================================================================

#[utoipa::path(
    post,
    path = "/api/timekeeping/rates",
    request_body = CreateBillingRateRequest,
    responses(
        (status = 201, description = "Billing rate created", body = BillingRate),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 422, description = "Validation error", body = ApiError),
    ),
    security(("bearer" = [])),
    tag = "Billing",
)]
pub async fn create_rate(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<CreateBillingRateRequest>,
) -> Result<(StatusCode, Json<BillingRate>), ApiError> {
    let app_id = extract_tenant(&claims)?;
    req.app_id = app_id;
    let rate = service::create_billing_rate(&state.pool, &req)
        .await
        .map_err(map_billing_error)?;
    Ok((StatusCode::CREATED, Json(rate)))
}

#[utoipa::path(
    get,
    path = "/api/timekeeping/rates",
    responses(
        (status = 200, description = "Billing rate list", body = Vec<BillingRate>),
        (status = 401, description = "Unauthorized", body = ApiError),
    ),
    security(("bearer" = [])),
    tag = "Billing",
)]
pub async fn list_rates(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
) -> Result<Json<PaginatedResponse<BillingRate>>, ApiError> {
    let app_id = extract_tenant(&claims)?;
    let rates = service::list_billing_rates(&state.pool, &app_id)
        .await
        .map_err(map_billing_error)?;
    let total = rates.len() as i64;
    Ok(Json(PaginatedResponse::new(rates, 1, total, total)))
}

#[utoipa::path(
    post,
    path = "/api/timekeeping/billing-runs",
    request_body = CreateBillingRunRequest,
    responses(
        (status = 201, description = "Billing run created", body = BillingRunResult),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 422, description = "No billable entries or validation error", body = ApiError),
    ),
    security(("bearer" = [])),
    tag = "Billing",
)]
pub async fn create_billing_run(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<CreateBillingRunRequest>,
) -> Result<(StatusCode, Json<BillingRunResult>), ApiError> {
    let app_id = extract_tenant(&claims)?;
    req.app_id = app_id;
    let result = service::create_billing_run(&state.pool, &req)
        .await
        .map_err(map_billing_error)?;
    Ok((StatusCode::CREATED, Json(result)))
}
