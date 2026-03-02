//! HTTP handlers for vendor bill CRUD, 3-way match, approval, void, and tax.
//!
//! POST /api/ap/bills              — create a vendor bill
//! GET  /api/ap/bills              — list bills for tenant (filter by vendor, voided)
//! GET  /api/ap/bills/:id          — get a single bill with its line items
//! POST /api/ap/bills/:id/match    — run 3-way match engine for a bill
//! POST /api/ap/bills/:id/approve  — approve a bill (enforces match policy)
//! POST /api/ap/bills/:id/void     — void a bill (requires reason)
//! POST /api/ap/bills/:id/tax-quote — quote tax for a bill draft

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    Extension, Json,
};
use security::VerifiedClaims;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::bills::{
    approve, service, void, ApproveBillRequest, BillError, CreateBillRequest, VendorBill,
    VendorBillWithLines, VoidBillRequest,
};
use crate::domain::r#match::{engine, MatchError, MatchOutcome, RunMatchRequest};
use crate::domain::tax::{self, ApTaxSnapshot, ZeroTaxProvider};
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

fn bill_error_response(e: BillError) -> (StatusCode, Json<ErrorBody>) {
    match e {
        BillError::NotFound(id) => (
            StatusCode::NOT_FOUND,
            Json(ErrorBody::new(
                "bill_not_found",
                &format!("Bill {} not found", id),
            )),
        ),
        BillError::VendorNotFound(id) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorBody::new(
                "vendor_not_found",
                &format!("Vendor {} not found or inactive", id),
            )),
        ),
        BillError::DuplicateInvoice(ref_) => (
            StatusCode::CONFLICT,
            Json(ErrorBody::new(
                "duplicate_invoice",
                &format!("Invoice '{}' already exists for this vendor", ref_),
            )),
        ),
        BillError::InvalidTransition { from, to } => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorBody::new(
                "invalid_transition",
                &format!("Cannot transition bill from '{}' to '{}'", from, to),
            )),
        ),
        BillError::EmptyLines => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorBody::new(
                "empty_lines",
                "Bill must have at least one line",
            )),
        ),
        BillError::Validation(msg) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorBody::new("validation_error", &msg)),
        ),
        BillError::MatchPolicyViolation(msg) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorBody::new("match_policy_violation", &msg)),
        ),
        BillError::TaxError(msg) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorBody::new("tax_error", &msg)),
        ),
        BillError::Database(e) => {
            tracing::error!("AP bills DB error: {}", e);
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
pub struct ListBillsQuery {
    /// Filter to a specific vendor
    pub vendor_id: Option<Uuid>,
    /// Include voided bills (default: false)
    #[serde(default)]
    pub include_voided: bool,
}

// ============================================================================
// Handlers
// ============================================================================

/// POST /api/ap/bills — create a vendor bill
pub async fn create_bill(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    headers: HeaderMap,
    Json(req): Json<CreateBillRequest>,
) -> Result<(StatusCode, Json<VendorBillWithLines>), (StatusCode, Json<ErrorBody>)> {
    let tenant_id = extract_tenant(&claims)?;
    let correlation_id = correlation_from_headers(&headers);

    let bill = service::create_bill(&state.pool, &tenant_id, &req, correlation_id)
        .await
        .map_err(bill_error_response)?;

    Ok((StatusCode::CREATED, Json(bill)))
}

/// GET /api/ap/bills/:bill_id — get a single bill with its line items
pub async fn get_bill(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(bill_id): Path<Uuid>,
) -> Result<Json<VendorBillWithLines>, (StatusCode, Json<ErrorBody>)> {
    let tenant_id = extract_tenant(&claims)?;

    let bill = service::get_bill(&state.pool, &tenant_id, bill_id)
        .await
        .map_err(bill_error_response)?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorBody::new(
                    "bill_not_found",
                    &format!("Bill {} not found", bill_id),
                )),
            )
        })?;

    Ok(Json(bill))
}

/// GET /api/ap/bills — list bills for tenant
pub async fn list_bills(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(query): Query<ListBillsQuery>,
) -> Result<Json<Vec<VendorBill>>, (StatusCode, Json<ErrorBody>)> {
    let tenant_id = extract_tenant(&claims)?;

    let bills = service::list_bills(
        &state.pool,
        &tenant_id,
        query.vendor_id,
        query.include_voided,
    )
    .await
    .map_err(bill_error_response)?;

    Ok(Json(bills))
}

// ============================================================================
// Approve / Void
// ============================================================================

/// POST /api/ap/bills/:bill_id/approve — approve a bill (enforces match policy)
pub async fn approve_bill(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    headers: HeaderMap,
    Path(bill_id): Path<Uuid>,
    Json(req): Json<ApproveBillRequest>,
) -> Result<Json<VendorBill>, (StatusCode, Json<ErrorBody>)> {
    let tenant_id = extract_tenant(&claims)?;
    let correlation_id = correlation_from_headers(&headers);
    let provider = ZeroTaxProvider;

    let bill = approve::approve_bill(
        &state.pool,
        &provider,
        &tenant_id,
        bill_id,
        &req,
        correlation_id,
    )
    .await
    .map_err(bill_error_response)?;

    Ok(Json(bill))
}

/// POST /api/ap/bills/:bill_id/void — void a bill (requires reason)
pub async fn void_bill(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    headers: HeaderMap,
    Path(bill_id): Path<Uuid>,
    Json(req): Json<VoidBillRequest>,
) -> Result<Json<VendorBill>, (StatusCode, Json<ErrorBody>)> {
    let tenant_id = extract_tenant(&claims)?;
    let correlation_id = correlation_from_headers(&headers);
    let provider = ZeroTaxProvider;

    let bill = void::void_bill(
        &state.pool,
        &provider,
        &tenant_id,
        bill_id,
        &req,
        correlation_id,
    )
    .await
    .map_err(bill_error_response)?;

    Ok(Json(bill))
}

// ============================================================================
// Tax quote
// ============================================================================

/// Request body for quoting tax on a bill draft.
#[derive(Debug, Deserialize)]
pub struct BillTaxQuoteRequest {
    /// Destination address (company's receiving location)
    pub ship_to: tax_core::TaxAddress,
    /// Origin address (vendor's shipping location)
    pub ship_from: tax_core::TaxAddress,
}

/// POST /api/ap/bills/:bill_id/tax-quote — quote tax for a bill draft
pub async fn quote_bill_tax(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    headers: HeaderMap,
    Path(bill_id): Path<Uuid>,
    Json(req): Json<BillTaxQuoteRequest>,
) -> Result<Json<ApTaxSnapshot>, (StatusCode, Json<ErrorBody>)> {
    let tenant_id = extract_tenant(&claims)?;
    let correlation_id = correlation_from_headers(&headers);

    // Fetch the bill and its lines to build the tax quote request
    let bill_with_lines = service::get_bill(&state.pool, &tenant_id, bill_id)
        .await
        .map_err(bill_error_response)?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorBody::new(
                    "bill_not_found",
                    &format!("Bill {} not found", bill_id),
                )),
            )
        })?;

    let line_items: Vec<tax_core::TaxLineItem> = bill_with_lines
        .lines
        .iter()
        .map(|l| tax_core::TaxLineItem {
            line_id: l.line_id.to_string(),
            description: l.description.clone(),
            amount_minor: l.line_total_minor,
            currency: bill_with_lines.bill.currency.clone(),
            tax_code: None,
            quantity: l.quantity,
        })
        .collect();

    let tax_req = tax_core::TaxQuoteRequest {
        tenant_id: tenant_id.clone(),
        invoice_id: bill_id.to_string(),
        customer_id: bill_with_lines.bill.vendor_id.to_string(),
        ship_to: req.ship_to,
        ship_from: req.ship_from,
        line_items,
        currency: bill_with_lines.bill.currency.clone(),
        invoice_date: bill_with_lines.bill.invoice_date,
        correlation_id,
    };

    let provider = ZeroTaxProvider;
    let snapshot =
        tax::quote_bill_tax(&state.pool, &provider, "zero", &tenant_id, bill_id, tax_req)
            .await
            .map_err(|e| {
                (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(ErrorBody::new("tax_error", &e.to_string())),
                )
            })?;

    Ok(Json(snapshot))
}

// ============================================================================
// 3-way match
// ============================================================================

fn match_error_response(e: MatchError) -> (StatusCode, Json<ErrorBody>) {
    match e {
        MatchError::BillNotFound(id) => (
            StatusCode::NOT_FOUND,
            Json(ErrorBody::new(
                "bill_not_found",
                &format!("Bill {} not found", id),
            )),
        ),
        MatchError::PoNotFound(id) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorBody::new(
                "po_not_found",
                &format!("PO {} not found", id),
            )),
        ),
        MatchError::InvalidBillStatus(s) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorBody::new(
                "invalid_bill_status",
                &format!(
                    "Bill status '{}' cannot be matched; must be 'open' or 'matched'",
                    s
                ),
            )),
        ),
        MatchError::NoMatchableLines => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorBody::new("no_matchable_lines", "Bill has no lines")),
        ),
        MatchError::Validation(msg) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorBody::new("validation_error", &msg)),
        ),
        MatchError::Database(e) => {
            tracing::error!("AP match DB error: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody::new("database_error", "Internal database error")),
            )
        }
    }
}

/// POST /api/ap/bills/:bill_id/match — run 3-way match engine for a bill
pub async fn match_bill(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    headers: HeaderMap,
    Path(bill_id): Path<Uuid>,
    Json(req): Json<RunMatchRequest>,
) -> Result<(StatusCode, Json<MatchOutcome>), (StatusCode, Json<ErrorBody>)> {
    let tenant_id = extract_tenant(&claims)?;
    let correlation_id = correlation_from_headers(&headers);

    let outcome = engine::run_match(&state.pool, &tenant_id, bill_id, &req, correlation_id)
        .await
        .map_err(match_error_response)?;

    Ok((StatusCode::OK, Json(outcome)))
}
