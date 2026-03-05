use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use security::VerifiedClaims;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use super::tenant::extract_tenant;
use crate::domain::bom_service::{self, BomError};
use crate::domain::guards::GuardError;
use crate::domain::models::*;
use crate::AppState;

// ============================================================================
// Error mapping
// ============================================================================

fn error_response(err: BomError) -> impl IntoResponse {
    match err {
        BomError::Guard(GuardError::NotFound(msg)) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": msg })),
        ),
        BomError::Guard(GuardError::Validation(msg)) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "validation_error", "message": msg })),
        ),
        BomError::Guard(GuardError::Conflict(msg)) => (
            StatusCode::CONFLICT,
            Json(json!({ "error": "conflict", "message": msg })),
        ),
        BomError::Guard(GuardError::CycleDetected) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "cycle_detected", "message": "Cycle detected in BOM structure" })),
        ),
        BomError::Guard(GuardError::Database(e)) => {
            tracing::error!(error = %e, "guard database error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Database error" })),
            )
        }
        BomError::Serialization(e) => {
            tracing::error!(error = %e, "serialization error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Serialization error" })),
            )
        }
        BomError::Database(ref e) => {
            // Check for unique constraint violations
            if let sqlx::Error::Database(dbe) = e {
                if dbe.code().as_deref() == Some("23505") {
                    return (
                        StatusCode::CONFLICT,
                        Json(json!({ "error": "duplicate", "message": dbe.message() })),
                    );
                }
                // Exclusion constraint violation (overlapping effectivity)
                if dbe.code().as_deref() == Some("23P01") {
                    return (
                        StatusCode::CONFLICT,
                        Json(json!({
                            "error": "effectivity_overlap",
                            "message": "Effective date range overlaps with an existing revision"
                        })),
                    );
                }
            }
            tracing::error!(error = %e, "database error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Database error" })),
            )
        }
    }
}

fn correlation_id() -> String {
    Uuid::new_v4().to_string()
}

// ============================================================================
// BOM Header
// ============================================================================

pub async fn post_bom(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(req): Json<CreateBomRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match bom_service::create_bom(&state.pool, &tenant_id, &req, &correlation_id(), None).await {
        Ok(header) => (StatusCode::CREATED, Json(json!(header))).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

pub async fn get_bom(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(bom_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match bom_service::get_bom(&state.pool, &tenant_id, bom_id).await {
        Ok(header) => Json(json!(header)).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

// ============================================================================
// Revisions
// ============================================================================

pub async fn post_revision(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(bom_id): Path<Uuid>,
    Json(req): Json<CreateRevisionRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match bom_service::create_revision(
        &state.pool,
        &tenant_id,
        bom_id,
        &req,
        &correlation_id(),
        None,
    )
    .await
    {
        Ok(rev) => (StatusCode::CREATED, Json(json!(rev))).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

pub async fn list_revisions(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(bom_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match bom_service::list_revisions(&state.pool, &tenant_id, bom_id).await {
        Ok(revs) => Json(json!(revs)).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

pub async fn post_effectivity(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(revision_id): Path<Uuid>,
    Json(req): Json<SetEffectivityRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match bom_service::set_effectivity(
        &state.pool,
        &tenant_id,
        revision_id,
        &req,
        &correlation_id(),
        None,
    )
    .await
    {
        Ok(rev) => Json(json!(rev)).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

// ============================================================================
// Lines
// ============================================================================

pub async fn post_line(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(revision_id): Path<Uuid>,
    Json(req): Json<AddLineRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match bom_service::add_line(
        &state.pool,
        &tenant_id,
        revision_id,
        &req,
        &correlation_id(),
        None,
    )
    .await
    {
        Ok(line) => (StatusCode::CREATED, Json(json!(line))).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

pub async fn put_line(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(line_id): Path<Uuid>,
    Json(req): Json<UpdateLineRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match bom_service::update_line(
        &state.pool,
        &tenant_id,
        line_id,
        &req,
        &correlation_id(),
        None,
    )
    .await
    {
        Ok(line) => Json(json!(line)).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

pub async fn delete_line(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(line_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match bom_service::remove_line(&state.pool, &tenant_id, line_id, &correlation_id(), None).await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

pub async fn get_lines(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(revision_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match bom_service::list_lines(&state.pool, &tenant_id, revision_id).await {
        Ok(lines) => Json(json!(lines)).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

// ============================================================================
// Explosion + Where-Used
// ============================================================================

pub async fn get_explosion(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(bom_id): Path<Uuid>,
    Query(query): Query<ExplosionQuery>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match bom_service::explode(&state.pool, &tenant_id, bom_id, &query).await {
        Ok(rows) => Json(json!(rows)).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

pub async fn get_where_used(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(item_id): Path<Uuid>,
    Query(query): Query<WhereUsedQuery>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match bom_service::where_used(&state.pool, &tenant_id, item_id, &query).await {
        Ok(rows) => Json(json!(rows)).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}
