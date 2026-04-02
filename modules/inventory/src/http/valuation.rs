//! Valuation snapshot HTTP handlers.
//!
//! Endpoints:
//!   POST /api/inventory/valuation-snapshots
//!     — build a point-in-time valuation snapshot from FIFO layers
//!
//!   GET /api/inventory/valuation-snapshots?warehouse_id=...
//!     — list snapshots (newest first), optionally filtered by warehouse
//!
//!   GET /api/inventory/valuation-snapshots/{id}
//!     — snapshot header + per-item lines
//!
//! Tenant derived from JWT VerifiedClaims.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use event_bus::TracingContext;
use platform_http_contracts::{ApiError, PaginatedResponse};
use security::VerifiedClaims;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use platform_sdk::extract_tenant;
use super::tenant::with_request_id;
use crate::{
    domain::valuation::{
        models::ValuationSnapshot,
        queries::{get_snapshot, get_snapshot_lines, list_snapshots},
        snapshot_service::CreateSnapshotRequest,
    },
    AppState,
};

// ============================================================================
// Query params
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct ListSnapshotsQuery {
    pub warehouse_id: Option<Uuid>,
    #[serde(default = "default_page")]
    pub page: i64,
    #[serde(default = "default_page_size")]
    pub page_size: i64,
}

fn default_page() -> i64 {
    1
}
fn default_page_size() -> i64 {
    50
}

// ============================================================================
// Handlers
// ============================================================================

#[utoipa::path(
    post,
    path = "/api/inventory/valuation-snapshots",
    tag = "Valuation",
    request_body = CreateSnapshotRequest,
    responses(
        (status = 201, description = "Valuation snapshot created", body = ValuationSnapshot),
        (status = 200, description = "Idempotency replay", body = ValuationSnapshot),
        (status = 409, description = "Idempotency key conflict", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn post_valuation_snapshot(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<CreateSnapshotRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    req.tenant_id = tenant_id;
    match crate::domain::valuation::snapshot_service::create_valuation_snapshot(&state.pool, &req)
        .await
    {
        Ok((result, false)) => (StatusCode::CREATED, Json(result)).into_response(),
        Ok((result, true)) => (StatusCode::OK, Json(result)).into_response(),
        Err(err) => {
            let api_err: ApiError = err.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/inventory/valuation-snapshots",
    tag = "Valuation",
    params(
        ("warehouse_id" = Option<Uuid>, Query, description = "Filter by warehouse"),
        ("limit" = Option<i64>, Query, description = "Page size (default 50, max 200)"),
        ("offset" = Option<i64>, Query, description = "Offset (default 0)"),
    ),
    responses(
        (status = 200, description = "Paginated snapshot list", body = PaginatedResponse<ValuationSnapshot>),
    ),
    security(("bearer" = [])),
)]
pub async fn list_valuation_snapshots(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(q): Query<ListSnapshotsQuery>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    let page_size = q.page_size.clamp(1, 200);
    let page = q.page.max(1);
    let offset = (page - 1) * page_size;

    match list_snapshots(&state.pool, &tenant_id, q.warehouse_id, page_size, offset).await {
        Ok(snapshots) => {
            let count = snapshots.len() as i64;
            // Use count as total_items since the query returns a windowed result;
            // for accurate totals, a COUNT(*) query would be needed. For now, use
            // count + offset as a lower bound.
            let total_items = offset + count;
            let resp = PaginatedResponse::new(snapshots, page, page_size, total_items);
            (StatusCode::OK, Json(resp)).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, tenant_id = %tenant_id, "database error listing valuation snapshots");
            with_request_id(ApiError::internal("Database error"), &tracing_ctx).into_response()
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/inventory/valuation-snapshots/{id}",
    tag = "Valuation",
    params(("id" = Uuid, Path, description = "Snapshot ID")),
    responses(
        (status = 200, description = "Snapshot with lines", body = serde_json::Value),
        (status = 404, description = "Snapshot not found", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn get_valuation_snapshot(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    let snapshot = match get_snapshot(&state.pool, &tenant_id, id).await {
        Ok(Some(s)) => s,
        Ok(None) => {
            return with_request_id(
                ApiError::not_found("Valuation snapshot not found"),
                &tracing_ctx,
            )
            .into_response();
        }
        Err(e) => {
            tracing::error!(error = %e, snapshot_id = %id, "database error fetching valuation snapshot");
            return with_request_id(ApiError::internal("Database error"), &tracing_ctx)
                .into_response();
        }
    };

    let lines = match get_snapshot_lines(&state.pool, id).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!(error = %e, snapshot_id = %id, "database error fetching valuation lines");
            return with_request_id(ApiError::internal("Database error"), &tracing_ctx)
                .into_response();
        }
    };

    (
        StatusCode::OK,
        Json(json!({
            "id": snapshot.id,
            "tenant_id": snapshot.tenant_id,
            "warehouse_id": snapshot.warehouse_id,
            "location_id": snapshot.location_id,
            "as_of": snapshot.as_of,
            "total_value_minor": snapshot.total_value_minor,
            "currency": snapshot.currency,
            "created_at": snapshot.created_at,
            "line_count": lines.len(),
            "lines": lines,
        })),
    )
        .into_response()
}
