//! Item revision HTTP handlers.
//!
//! Endpoints:
//!   POST  /api/inventory/items/:item_id/revisions                         — create revision
//!   POST  /api/inventory/items/:item_id/revisions/:revision_id/activate   — activate revision
//!   PUT   /api/inventory/items/:item_id/revisions/:revision_id/policy-flags — update draft policy flags
//!   GET   /api/inventory/items/:item_id/revisions/at                      — query at time T
//!   GET   /api/inventory/items/:item_id/revisions                         — list all revisions

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use chrono::{DateTime, Utc};
use security::VerifiedClaims;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use super::tenant::extract_tenant;
use crate::{
    domain::revisions::{
        activate_revision, create_revision, list_revisions, revision_at, update_revision_policy,
        ActivateRevisionRequest, CreateRevisionRequest, RevisionError, UpdateRevisionPolicyRequest,
    },
    AppState,
};

// ============================================================================
// Error mapping
// ============================================================================

fn revision_error_response(err: RevisionError) -> impl IntoResponse {
    match err {
        RevisionError::ItemNotFound | RevisionError::RevisionNotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": err.to_string() })),
        ),
        RevisionError::ItemInactive => (
            StatusCode::CONFLICT,
            Json(json!({ "error": "item_inactive", "message": err.to_string() })),
        ),
        RevisionError::AlreadyActivated => (
            StatusCode::CONFLICT,
            Json(json!({ "error": "already_activated", "message": err.to_string() })),
        ),
        RevisionError::PolicyLockedOnActivatedRevision => (
            StatusCode::CONFLICT,
            Json(json!({ "error": "policy_locked", "message": err.to_string() })),
        ),
        RevisionError::OverlappingWindow => (
            StatusCode::CONFLICT,
            Json(json!({ "error": "overlapping_window", "message": err.to_string() })),
        ),
        RevisionError::ConflictingIdempotencyKey => (
            StatusCode::CONFLICT,
            Json(json!({ "error": "idempotency_conflict", "message": err.to_string() })),
        ),
        RevisionError::Validation(msg) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "validation_error", "message": msg })),
        ),
        RevisionError::Serialization(e) => {
            tracing::error!(error = %e, "revision serialization error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Serialization error" })),
            )
        }
        RevisionError::Database(e) => {
            tracing::error!(error = %e, "revision database error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Database error" })),
            )
        }
    }
}

// ============================================================================
// Handlers
// ============================================================================

/// POST /api/inventory/items/:item_id/revisions
pub async fn post_create_revision(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(item_id): Path<Uuid>,
    Json(mut req): Json<CreateRevisionRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    req.tenant_id = tenant_id;
    req.item_id = item_id;

    match create_revision(&state.pool, &req).await {
        Ok((rev, false)) => (StatusCode::CREATED, Json(json!(rev))).into_response(),
        Ok((rev, true)) => (StatusCode::OK, Json(json!(rev))).into_response(),
        Err(e) => revision_error_response(e).into_response(),
    }
}

/// POST /api/inventory/items/:item_id/revisions/:revision_id/activate
pub async fn post_activate_revision(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path((item_id, revision_id)): Path<(Uuid, Uuid)>,
    Json(mut req): Json<ActivateRevisionRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    req.tenant_id = tenant_id;

    match activate_revision(&state.pool, item_id, revision_id, &req).await {
        Ok((rev, false)) => (StatusCode::OK, Json(json!(rev))).into_response(),
        Ok((rev, true)) => (StatusCode::OK, Json(json!(rev))).into_response(),
        Err(e) => revision_error_response(e).into_response(),
    }
}

/// PUT /api/inventory/items/:item_id/revisions/:revision_id/policy-flags
pub async fn put_revision_policy(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path((item_id, revision_id)): Path<(Uuid, Uuid)>,
    Json(mut req): Json<UpdateRevisionPolicyRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    req.tenant_id = tenant_id;

    match update_revision_policy(&state.pool, item_id, revision_id, &req).await {
        Ok((rev, false)) => (StatusCode::OK, Json(json!(rev))).into_response(),
        Ok((rev, true)) => (StatusCode::OK, Json(json!(rev))).into_response(),
        Err(e) => revision_error_response(e).into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct RevisionAtQuery {
    pub t: Option<DateTime<Utc>>,
}

/// GET /api/inventory/items/:item_id/revisions/at?t=...
pub async fn get_revision_at(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(item_id): Path<Uuid>,
    Query(query): Query<RevisionAtQuery>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    let at = query.t.unwrap_or_else(Utc::now);

    match revision_at(&state.pool, &tenant_id, item_id, at).await {
        Ok(Some(rev)) => (StatusCode::OK, Json(json!(rev))).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "No revision effective at requested time" })),
        )
            .into_response(),
        Err(e) => revision_error_response(e).into_response(),
    }
}

/// GET /api/inventory/items/:item_id/revisions
pub async fn get_list_revisions(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(item_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    match list_revisions(&state.pool, &tenant_id, item_id).await {
        Ok(revs) => (StatusCode::OK, Json(json!(revs))).into_response(),
        Err(e) => revision_error_response(e).into_response(),
    }
}
