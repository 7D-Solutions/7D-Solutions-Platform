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
use security::VerifiedClaims;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    domain::valuation::{
        queries::{get_snapshot, get_snapshot_lines, list_snapshots},
        snapshot_service::{create_valuation_snapshot, CreateSnapshotRequest, SnapshotError},
    },
    AppState,
};

// ============================================================================
// Error mapping
// ============================================================================

fn snapshot_error_response(err: SnapshotError) -> impl IntoResponse {
    match err {
        SnapshotError::MissingTenant | SnapshotError::MissingIdempotencyKey => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "validation_error", "message": err.to_string() })),
        )
            .into_response(),

        SnapshotError::ConcurrentSnapshot => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "concurrent_snapshot",
                "message": err.to_string()
            })),
        )
            .into_response(),

        SnapshotError::ConflictingIdempotencyKey => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "idempotency_conflict",
                "message": err.to_string()
            })),
        )
            .into_response(),

        SnapshotError::Serialization(e) => {
            tracing::error!(error = %e, "serialization error in valuation snapshot");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Serialization error" })),
            )
                .into_response()
        }

        SnapshotError::Database(e) => {
            tracing::error!(error = %e, "database error creating valuation snapshot");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Database error" })),
            )
                .into_response()
        }
    }
}

// ============================================================================
// Query params
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct ListSnapshotsQuery {
    pub warehouse_id: Option<Uuid>,
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_limit() -> i64 {
    50
}

// ============================================================================
// Handlers
// ============================================================================

/// POST /api/inventory/valuation-snapshots
///
/// Creates a valuation snapshot from remaining FIFO layers as-of `req.as_of`.
/// Tenant derived from JWT VerifiedClaims — body tenant_id is overridden.
/// Returns 201 on creation; 200 on idempotent replay.
pub async fn post_valuation_snapshot(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<CreateSnapshotRequest>,
) -> impl IntoResponse {
    let tenant_id = match &claims {
        Some(Extension(c)) => c.tenant_id.to_string(),
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "unauthorized", "message": "Missing or invalid authentication" })),
            )
                .into_response();
        }
    };
    req.tenant_id = tenant_id;
    match create_valuation_snapshot(&state.pool, &req).await {
        Ok((result, false)) => (StatusCode::CREATED, Json(result)).into_response(),
        Ok((result, true)) => (StatusCode::OK, Json(result)).into_response(),
        Err(err) => snapshot_error_response(err).into_response(),
    }
}

/// GET /api/inventory/valuation-snapshots?warehouse_id=...&limit=...&offset=...
///
/// Lists snapshots for the tenant (from JWT), newest first.  Optional
/// `warehouse_id` narrows to one warehouse.  `limit` defaults to 50
/// (max 200); `offset` defaults to 0.
///
/// Returns `{ "tenant_id", "snapshots": [...] }`.
pub async fn list_valuation_snapshots(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(q): Query<ListSnapshotsQuery>,
) -> impl IntoResponse {
    let tenant_id = match &claims {
        Some(Extension(c)) => c.tenant_id.to_string(),
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "unauthorized", "message": "Missing or invalid authentication" })),
            )
                .into_response();
        }
    };

    let limit = q.limit.clamp(1, 200);
    let offset = q.offset.max(0);

    match list_snapshots(&state.pool, &tenant_id, q.warehouse_id, limit, offset).await {
        Ok(snapshots) => (
            StatusCode::OK,
            Json(json!({
                "tenant_id": tenant_id,
                "warehouse_id": q.warehouse_id,
                "limit": limit,
                "offset": offset,
                "count": snapshots.len(),
                "snapshots": snapshots,
            })),
        )
            .into_response(),
        Err(e) => {
            tracing::error!(error = %e, tenant_id = %tenant_id, "database error listing valuation snapshots");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Database error" })),
            )
                .into_response()
        }
    }
}

/// GET /api/inventory/valuation-snapshots/{id}
///
/// Returns the snapshot header and all per-item lines, tenant-scoped (from JWT).
/// Returns 404 when the snapshot does not exist or belongs to another tenant.
///
/// Response: `{ snapshot header fields..., "lines": [...] }`.
pub async fn get_valuation_snapshot(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
) -> impl IntoResponse {
    let tenant_id = match &claims {
        Some(Extension(c)) => c.tenant_id.to_string(),
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "unauthorized", "message": "Missing or invalid authentication" })),
            )
                .into_response();
        }
    };

    let snapshot = match get_snapshot(&state.pool, &tenant_id, id).await {
        Ok(Some(s)) => s,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "not_found", "message": "Valuation snapshot not found" })),
            )
                .into_response();
        }
        Err(e) => {
            tracing::error!(error = %e, snapshot_id = %id, "database error fetching valuation snapshot");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Database error" })),
            )
                .into_response();
        }
    };

    let lines = match get_snapshot_lines(&state.pool, id).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!(error = %e, snapshot_id = %id, "database error fetching valuation lines");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Database error" })),
            )
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
