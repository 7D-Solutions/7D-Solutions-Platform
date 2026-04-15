//! HTTP handlers for payment terms CRUD and bill assignment.
//!
//! POST /api/ap/payment-terms              — create payment terms
//! GET  /api/ap/payment-terms              — list payment terms for tenant
//! GET  /api/ap/payment-terms/:id          — get a single payment terms record
//! POST /api/ap/bills/:bill_id/assign-terms — assign terms to a bill

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Extension, Json,
};
use event_bus::TracingContext;
use platform_http_contracts::{ApiError, PaginatedResponse};
use security::VerifiedClaims;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::payment_terms::{
    service, AssignTermsRequest, CreatePaymentTermsRequest, UpdatePaymentTermsRequest,
};
use crate::http::tenant::with_request_id;
use crate::AppState;
use platform_sdk::extract_tenant;

// ============================================================================
// Shared helpers
// ============================================================================

fn correlation_from_headers(headers: &HeaderMap) -> String {
    headers
        .get("x-correlation-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown")
        .to_string()
}

// ============================================================================
// Query params
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct ListTermsQuery {
    #[serde(default)]
    pub include_inactive: bool,
}

// ============================================================================
// Handlers
// ============================================================================

#[utoipa::path(post, path = "/api/ap/payment-terms", tag = "Payment Terms",
    request_body = CreatePaymentTermsRequest,
    responses((status = 201, description = "Terms created", body = crate::domain::payment_terms::PaymentTerms)), security(("bearer" = [])))]
pub async fn create_terms(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    headers: HeaderMap,
    Json(req): Json<CreatePaymentTermsRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let correlation_id = correlation_from_headers(&headers);

    match service::create_terms(&state.pool, &tenant_id, &req, correlation_id).await {
        Ok(terms) => (StatusCode::CREATED, Json(terms)).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(get, path = "/api/ap/payment-terms/{term_id}", tag = "Payment Terms",
    params(("term_id" = Uuid, Path)), responses((status = 200, description = "Terms details", body = crate::domain::payment_terms::PaymentTerms)),
    security(("bearer" = [])))]
pub async fn get_terms(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(term_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match service::get_terms(&state.pool, &tenant_id, term_id).await {
        Ok(Some(terms)) => Json(terms).into_response(),
        Ok(None) => with_request_id(
            ApiError::not_found(format!("Payment terms {} not found", term_id)),
            &tracing_ctx,
        )
        .into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(get, path = "/api/ap/payment-terms", tag = "Payment Terms",
    responses((status = 200, description = "Terms list", body = PaginatedResponse<crate::domain::payment_terms::PaymentTerms>)),
    security(("bearer" = [])))]
pub async fn list_terms(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Query(query): Query<ListTermsQuery>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match service::list_terms(&state.pool, &tenant_id, query.include_inactive).await {
        Ok(terms) => {
            let total = terms.len() as i64;
            let resp = PaginatedResponse::new(terms, 1, total, total);
            Json(resp).into_response()
        }
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(put, path = "/api/ap/payment-terms/{term_id}", tag = "Payment Terms",
    params(("term_id" = Uuid, Path)), request_body = UpdatePaymentTermsRequest,
    responses((status = 200, description = "Terms updated", body = crate::domain::payment_terms::PaymentTerms)), security(("bearer" = [])))]
pub async fn update_terms(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(term_id): Path<Uuid>,
    Json(req): Json<UpdatePaymentTermsRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match service::update_terms(&state.pool, &tenant_id, term_id, &req).await {
        Ok(terms) => Json(terms).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(post, path = "/api/ap/bills/{bill_id}/assign-terms", tag = "Payment Terms",
    params(("bill_id" = Uuid, Path)), request_body = AssignTermsRequest,
    responses((status = 200, description = "Terms assigned", body = serde_json::Value)), security(("bearer" = [])))]
pub async fn assign_terms(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(bill_id): Path<Uuid>,
    Json(req): Json<AssignTermsRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match service::assign_terms_to_bill(&state.pool, &tenant_id, bill_id, req.term_id).await {
        Ok(result) => Json(result).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}
