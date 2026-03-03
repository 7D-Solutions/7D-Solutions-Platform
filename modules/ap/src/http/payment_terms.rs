//! HTTP handlers for payment terms CRUD and bill assignment.
//!
//! POST /api/ap/payment-terms              — create payment terms
//! GET  /api/ap/payment-terms              — list payment terms for tenant
//! GET  /api/ap/payment-terms/:id          — get a single payment terms record
//! POST /api/ap/bills/:bill_id/assign-terms — assign terms to a bill

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    Extension, Json,
};
use security::VerifiedClaims;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::payment_terms::{
    service, AssignTermsRequest, AssignTermsResult, CreatePaymentTermsRequest, PaymentTerms,
    PaymentTermsError, UpdatePaymentTermsRequest,
};
use crate::http::admin_types::ErrorBody;
use crate::http::tenant::extract_tenant;
use crate::AppState;

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

fn terms_error_response(e: PaymentTermsError) -> (StatusCode, Json<ErrorBody>) {
    match e {
        PaymentTermsError::NotFound(id) => (
            StatusCode::NOT_FOUND,
            Json(ErrorBody::new(
                "payment_terms_not_found",
                &format!("Payment terms {} not found", id),
            )),
        ),
        PaymentTermsError::BillNotFound(id) => (
            StatusCode::NOT_FOUND,
            Json(ErrorBody::new(
                "bill_not_found",
                &format!("Bill {} not found", id),
            )),
        ),
        PaymentTermsError::DuplicateTermCode(code) => (
            StatusCode::CONFLICT,
            Json(ErrorBody::new(
                "duplicate_term_code",
                &format!("Term code '{}' already exists", code),
            )),
        ),
        PaymentTermsError::DuplicateIdempotencyKey(key) => (
            StatusCode::CONFLICT,
            Json(ErrorBody::new(
                "duplicate_idempotency_key",
                &format!("Idempotency key '{}' already used", key),
            )),
        ),
        PaymentTermsError::Validation(msg) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorBody::new("validation_error", &msg)),
        ),
        PaymentTermsError::Database(e) => {
            tracing::error!("AP payment_terms DB error: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody::new("database_error", "Internal database error")),
            )
        }
    }
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

/// POST /api/ap/payment-terms — create payment terms
pub async fn create_terms(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    headers: HeaderMap,
    Json(req): Json<CreatePaymentTermsRequest>,
) -> Result<(StatusCode, Json<PaymentTerms>), (StatusCode, Json<ErrorBody>)> {
    let tenant_id = extract_tenant(&claims)?;
    let correlation_id = correlation_from_headers(&headers);

    let terms = service::create_terms(&state.pool, &tenant_id, &req, correlation_id)
        .await
        .map_err(terms_error_response)?;

    Ok((StatusCode::CREATED, Json(terms)))
}

/// GET /api/ap/payment-terms/:term_id — get a single payment terms record
pub async fn get_terms(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(term_id): Path<Uuid>,
) -> Result<Json<PaymentTerms>, (StatusCode, Json<ErrorBody>)> {
    let tenant_id = extract_tenant(&claims)?;

    let terms = service::get_terms(&state.pool, &tenant_id, term_id)
        .await
        .map_err(terms_error_response)?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorBody::new(
                    "payment_terms_not_found",
                    &format!("Payment terms {} not found", term_id),
                )),
            )
        })?;

    Ok(Json(terms))
}

/// GET /api/ap/payment-terms — list payment terms for tenant
pub async fn list_terms(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(query): Query<ListTermsQuery>,
) -> Result<Json<Vec<PaymentTerms>>, (StatusCode, Json<ErrorBody>)> {
    let tenant_id = extract_tenant(&claims)?;

    let terms = service::list_terms(&state.pool, &tenant_id, query.include_inactive)
        .await
        .map_err(terms_error_response)?;

    Ok(Json(terms))
}

/// PUT /api/ap/payment-terms/:term_id — update payment terms
pub async fn update_terms(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(term_id): Path<Uuid>,
    Json(req): Json<UpdatePaymentTermsRequest>,
) -> Result<Json<PaymentTerms>, (StatusCode, Json<ErrorBody>)> {
    let tenant_id = extract_tenant(&claims)?;

    let terms = service::update_terms(&state.pool, &tenant_id, term_id, &req)
        .await
        .map_err(terms_error_response)?;

    Ok(Json(terms))
}

/// POST /api/ap/bills/:bill_id/assign-terms — assign terms to a bill
pub async fn assign_terms(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(bill_id): Path<Uuid>,
    Json(req): Json<AssignTermsRequest>,
) -> Result<Json<AssignTermsResult>, (StatusCode, Json<ErrorBody>)> {
    let tenant_id = extract_tenant(&claims)?;

    let result = service::assign_terms_to_bill(&state.pool, &tenant_id, bill_id, req.term_id)
        .await
        .map_err(terms_error_response)?;

    Ok(Json(result))
}
