//! HTTP handlers for purchase order CRUD and lifecycle.
//!
//! POST /api/ap/pos                    — create a draft PO with line items
//! GET  /api/ap/pos                    — list POs for tenant
//! GET  /api/ap/pos/:po_id             — get a single PO with its lines
//! PUT  /api/ap/pos/:po_id/lines       — replace all lines on a draft PO (idempotent)
//! POST /api/ap/pos/:po_id/approve     — approve a draft PO (idempotent)

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

use crate::domain::po::{
    approve, queries, service, ApprovePoRequest, CreatePoRequest, UpdatePoLinesRequest,
};
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
pub struct ListPosQuery {
    /// Filter to a specific vendor
    pub vendor_id: Option<Uuid>,
    /// Filter by status (draft, approved, closed, cancelled)
    pub status: Option<String>,
}

// ============================================================================
// Handlers
// ============================================================================

/// POST /api/ap/pos — create a draft PO with line items
pub async fn create_po(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    headers: HeaderMap,
    Json(req): Json<CreatePoRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let correlation_id = correlation_from_headers(&headers);

    match service::create_po(&state.pool, &tenant_id, &req, correlation_id).await {
        Ok(po) => (StatusCode::CREATED, Json(po)).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

/// GET /api/ap/pos/:po_id — get a single PO with its lines
pub async fn get_po(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(po_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match queries::get_po(&state.pool, &tenant_id, po_id).await {
        Ok(Some(po)) => Json(po).into_response(),
        Ok(None) => with_request_id(
            ApiError::not_found(format!("PO {} not found", po_id)),
            &tracing_ctx,
        )
        .into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

/// GET /api/ap/pos — list POs for tenant (optionally filtered by vendor_id or status)
pub async fn list_pos(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Query(query): Query<ListPosQuery>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match queries::list_pos(
        &state.pool,
        &tenant_id,
        query.vendor_id,
        query.status.as_deref(),
    )
    .await
    {
        Ok(pos) => {
            let total = pos.len() as i64;
            let resp = PaginatedResponse::new(pos, 1, total, total);
            Json(resp).into_response()
        }
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

/// PUT /api/ap/pos/:po_id/lines — replace all lines on a draft PO (idempotent)
pub async fn update_po_lines(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(po_id): Path<Uuid>,
    Json(req): Json<UpdatePoLinesRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match service::update_po_lines(&state.pool, &tenant_id, po_id, &req).await {
        Ok(po) => Json(po).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

/// POST /api/ap/pos/:po_id/approve — approve a draft PO (idempotent)
///
/// Transitions the PO from draft → approved and emits ap.po_approved.
/// If the PO is already approved, returns 200 with the current state (no re-emit).
/// Returns 422 if the PO is in a terminal state that cannot be approved.
pub async fn approve_po(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    headers: HeaderMap,
    Path(po_id): Path<Uuid>,
    Json(req): Json<ApprovePoRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let correlation_id = correlation_from_headers(&headers);

    match approve::approve_po(&state.pool, &tenant_id, po_id, &req, correlation_id).await {
        Ok(po) => Json(po).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}
