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
use crate::domain::models::*;
use crate::domain::service::{self, QiError};
use crate::AppState;

// ============================================================================
// Error mapping
// ============================================================================

fn error_response(err: QiError) -> impl IntoResponse {
    match err {
        QiError::NotFound(msg) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": msg })),
        ),
        QiError::Validation(msg) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "validation_error", "message": msg })),
        ),
        QiError::Serialization(e) => {
            tracing::error!(error = %e, "serialization error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Serialization error" })),
            )
        }
        QiError::Database(ref e) => {
            if let sqlx::Error::Database(dbe) = e {
                if dbe.code().as_deref() == Some("23505") {
                    return (
                        StatusCode::CONFLICT,
                        Json(json!({ "error": "duplicate", "message": dbe.message() })),
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
// Inspection Plans
// ============================================================================

pub async fn post_inspection_plan(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(req): Json<CreateInspectionPlanRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match service::create_inspection_plan(
        &state.pool,
        &tenant_id,
        &req,
        &correlation_id(),
        None,
    )
    .await
    {
        Ok(plan) => (StatusCode::CREATED, Json(json!(plan))).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

pub async fn get_inspection_plan(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(plan_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match service::get_inspection_plan(&state.pool, &tenant_id, plan_id).await {
        Ok(plan) => Json(json!(plan)).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

pub async fn post_activate_plan(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(plan_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match service::activate_plan(&state.pool, &tenant_id, plan_id).await {
        Ok(plan) => Json(json!(plan)).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

// ============================================================================
// Receiving Inspections
// ============================================================================

pub async fn post_receiving_inspection(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(req): Json<CreateReceivingInspectionRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match service::create_receiving_inspection(
        &state.pool,
        &tenant_id,
        &req,
        &correlation_id(),
        None,
    )
    .await
    {
        Ok(inspection) => (StatusCode::CREATED, Json(json!(inspection))).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

pub async fn get_inspection(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(inspection_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match service::get_inspection(&state.pool, &tenant_id, inspection_id).await {
        Ok(i) => Json(json!(i)).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

// ============================================================================
// Disposition transitions
// ============================================================================

pub async fn post_hold_inspection(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(inspection_id): Path<Uuid>,
    Json(req): Json<DispositionTransitionRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match service::hold_inspection(
        &state.pool,
        &tenant_id,
        inspection_id,
        req.inspector_id,
        req.reason.as_deref(),
        &correlation_id(),
        None,
    )
    .await
    {
        Ok(i) => Json(json!(i)).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

pub async fn post_release_inspection(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(inspection_id): Path<Uuid>,
    Json(req): Json<DispositionTransitionRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match service::release_inspection(
        &state.pool,
        &tenant_id,
        inspection_id,
        req.inspector_id,
        req.reason.as_deref(),
        &correlation_id(),
        None,
    )
    .await
    {
        Ok(i) => Json(json!(i)).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

pub async fn post_accept_inspection(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(inspection_id): Path<Uuid>,
    Json(req): Json<DispositionTransitionRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match service::accept_inspection(
        &state.pool,
        &tenant_id,
        inspection_id,
        req.inspector_id,
        req.reason.as_deref(),
        &correlation_id(),
        None,
    )
    .await
    {
        Ok(i) => Json(json!(i)).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

pub async fn post_reject_inspection(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(inspection_id): Path<Uuid>,
    Json(req): Json<DispositionTransitionRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match service::reject_inspection(
        &state.pool,
        &tenant_id,
        inspection_id,
        req.inspector_id,
        req.reason.as_deref(),
        &correlation_id(),
        None,
    )
    .await
    {
        Ok(i) => Json(json!(i)).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

// ============================================================================
// Queries
// ============================================================================

pub async fn get_inspections_by_part_rev(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(q): Query<InspectionsByPartRevQuery>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match service::list_inspections_by_part_rev(
        &state.pool,
        &tenant_id,
        q.part_id,
        q.part_revision.as_deref(),
    )
    .await
    {
        Ok(rows) => Json(json!(rows)).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}

pub async fn get_inspections_by_receipt(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(q): Query<InspectionsByReceiptQuery>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match service::list_inspections_by_receipt(&state.pool, &tenant_id, q.receipt_id).await {
        Ok(rows) => Json(json!(rows)).into_response(),
        Err(e) => error_response(e).into_response(),
    }
}
