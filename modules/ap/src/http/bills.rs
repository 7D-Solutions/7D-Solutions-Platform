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
    response::IntoResponse,
    Extension, Json,
};
use event_bus::TracingContext;
use platform_http_contracts::{ApiError, PaginatedResponse};
use security::VerifiedClaims;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::bills::{
    approve, service, void, ApproveBillRequest, CreateBillRequest, VoidBillRequest,
};
use crate::domain::r#match::{engine, RunMatchRequest};
use crate::domain::tax::{self, ZeroTaxProvider};
use crate::http::tenant::{extract_tenant, with_request_id};
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
    tracing_ctx: Option<Extension<TracingContext>>,
    headers: HeaderMap,
    Json(req): Json<CreateBillRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let correlation_id = correlation_from_headers(&headers);

    match service::create_bill(&state.pool, &tenant_id, &req, correlation_id).await {
        Ok(bill) => (StatusCode::CREATED, Json(bill)).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

/// GET /api/ap/bills/:bill_id — get a single bill with its line items
pub async fn get_bill(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(bill_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match service::get_bill(&state.pool, &tenant_id, bill_id).await {
        Ok(Some(bill)) => Json(bill).into_response(),
        Ok(None) => with_request_id(
            ApiError::not_found(format!("Bill {} not found", bill_id)),
            &tracing_ctx,
        )
        .into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

/// GET /api/ap/bills — list bills for tenant
pub async fn list_bills(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Query(query): Query<ListBillsQuery>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match service::list_bills(
        &state.pool,
        &tenant_id,
        query.vendor_id,
        query.include_voided,
    )
    .await
    {
        Ok(bills) => {
            let total = bills.len() as i64;
            let resp = PaginatedResponse::new(bills, 1, total, total);
            Json(resp).into_response()
        }
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

// ============================================================================
// Approve / Void
// ============================================================================

/// POST /api/ap/bills/:bill_id/approve — approve a bill (enforces match policy)
pub async fn approve_bill(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    headers: HeaderMap,
    Path(bill_id): Path<Uuid>,
    Json(req): Json<ApproveBillRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let correlation_id = correlation_from_headers(&headers);
    let provider = ZeroTaxProvider;

    match approve::approve_bill(
        &state.pool,
        &provider,
        &tenant_id,
        bill_id,
        &req,
        correlation_id,
    )
    .await
    {
        Ok(bill) => Json(bill).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

/// POST /api/ap/bills/:bill_id/void — void a bill (requires reason)
pub async fn void_bill(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    headers: HeaderMap,
    Path(bill_id): Path<Uuid>,
    Json(req): Json<VoidBillRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let correlation_id = correlation_from_headers(&headers);
    let provider = ZeroTaxProvider;

    match void::void_bill(
        &state.pool,
        &provider,
        &tenant_id,
        bill_id,
        &req,
        correlation_id,
    )
    .await
    {
        Ok(bill) => Json(bill).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
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
    tracing_ctx: Option<Extension<TracingContext>>,
    headers: HeaderMap,
    Path(bill_id): Path<Uuid>,
    Json(req): Json<BillTaxQuoteRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let correlation_id = correlation_from_headers(&headers);

    // Fetch the bill and its lines to build the tax quote request
    let bill_with_lines = match service::get_bill(&state.pool, &tenant_id, bill_id).await {
        Ok(Some(b)) => b,
        Ok(None) => {
            return with_request_id(
                ApiError::not_found(format!("Bill {} not found", bill_id)),
                &tracing_ctx,
            )
            .into_response()
        }
        Err(e) => return with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    };

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
    match tax::quote_bill_tax(&state.pool, &provider, "zero", &tenant_id, bill_id, tax_req).await {
        Ok(snapshot) => Json(snapshot).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

// ============================================================================
// 3-way match
// ============================================================================

/// POST /api/ap/bills/:bill_id/match — run 3-way match engine for a bill
pub async fn match_bill(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    headers: HeaderMap,
    Path(bill_id): Path<Uuid>,
    Json(req): Json<RunMatchRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let correlation_id = correlation_from_headers(&headers);

    match engine::run_match(&state.pool, &tenant_id, bill_id, &req, correlation_id).await {
        Ok(outcome) => (StatusCode::OK, Json(outcome)).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}
