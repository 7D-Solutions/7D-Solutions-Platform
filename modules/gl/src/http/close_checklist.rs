//! Pre-Close Checklist & Approvals API Routes (Phase 31, bd-bfa3)
//!
//! Provides HTTP endpoints for pre-close checklist management:
//! - Create checklist items for a period
//! - Complete / waive items (with reason)
//! - Record approval signoffs (idempotent)
//! - Query checklist and approval status

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use chrono::{DateTime, Utc};
use event_bus::TracingContext;
use platform_http_contracts::{ApiError, PaginatedResponse};
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

use super::auth::with_request_id;
use crate::AppState;
use platform_sdk::extract_tenant;

// ============================================================
// CHECKLIST TYPES
// ============================================================

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CreateChecklistItemRequest {
    pub label: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ChecklistItemResponse {
    pub id: Uuid,
    pub tenant_id: String,
    pub period_id: Uuid,
    pub label: String,
    pub status: String,
    pub completed_by: Option<String>,
    pub completed_at: Option<DateTime<Utc>>,
    pub waive_reason: Option<String>,
}

use crate::repos::checklist_repo;

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CompleteChecklistItemRequest {
    pub completed_by: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct WaiveChecklistItemRequest {
    pub completed_by: String,
    pub waive_reason: String,
}

// ============================================================
// CHECKLIST HANDLERS
// ============================================================

/// POST /api/gl/periods/{period_id}/checklist — add a checklist item
#[utoipa::path(post, path = "/api/gl/periods/{period_id}/checklist", tag = "Close Checklist",
    params(("period_id" = Uuid, Path, description = "Accounting period ID")),
    request_body = CreateChecklistItemRequest,
    responses((status = 201, description = "Checklist item created", body = ChecklistItemResponse)),
    security(("bearer" = [])))]
pub async fn create_checklist_item(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path(period_id): Path<Uuid>,
    Json(request): Json<CreateChecklistItemRequest>,
) -> Result<(StatusCode, Json<ChecklistItemResponse>), ApiError> {
    let tenant_id = extract_tenant(&claims).map_err(|e| with_request_id(e, &ctx))?;

    let row = checklist_repo::create_checklist_item(
        &app_state.pool,
        &tenant_id,
        period_id,
        &request.label,
    )
    .await
    .map_err(|e| {
        with_request_id(
            ApiError::internal(format!("Failed to create checklist item: {}", e)),
            &ctx,
        )
    })?;

    Ok((StatusCode::CREATED, Json(to_checklist_response(row))))
}

/// POST /api/gl/periods/{period_id}/checklist/{item_id}/complete
#[utoipa::path(post, path = "/api/gl/periods/{period_id}/checklist/{item_id}/complete", tag = "Close Checklist",
    params(
        ("period_id" = Uuid, Path, description = "Accounting period ID"),
        ("item_id" = Uuid, Path, description = "Checklist item ID"),
    ),
    request_body = CompleteChecklistItemRequest,
    responses((status = 200, description = "Checklist item completed", body = ChecklistItemResponse)),
    security(("bearer" = [])))]
pub async fn complete_checklist_item(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path((period_id, item_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<CompleteChecklistItemRequest>,
) -> Result<Json<ChecklistItemResponse>, ApiError> {
    let tenant_id = extract_tenant(&claims).map_err(|e| with_request_id(e, &ctx))?;

    let row = checklist_repo::complete_checklist_item(
        &app_state.pool,
        &request.completed_by,
        item_id,
        period_id,
        &tenant_id,
    )
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "Database error");
        with_request_id(ApiError::internal("Internal database error"), &ctx)
    })?
    .ok_or_else(|| {
        with_request_id(
            ApiError::not_found(format!("Checklist item {} not found", item_id)),
            &ctx,
        )
    })?;

    Ok(Json(to_checklist_response(row)))
}

/// POST /api/gl/periods/{period_id}/checklist/{item_id}/waive
#[utoipa::path(post, path = "/api/gl/periods/{period_id}/checklist/{item_id}/waive", tag = "Close Checklist",
    params(
        ("period_id" = Uuid, Path, description = "Accounting period ID"),
        ("item_id" = Uuid, Path, description = "Checklist item ID"),
    ),
    request_body = WaiveChecklistItemRequest,
    responses((status = 200, description = "Checklist item waived", body = ChecklistItemResponse)),
    security(("bearer" = [])))]
pub async fn waive_checklist_item(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path((period_id, item_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<WaiveChecklistItemRequest>,
) -> Result<Json<ChecklistItemResponse>, ApiError> {
    let tenant_id = extract_tenant(&claims).map_err(|e| with_request_id(e, &ctx))?;

    let row = checklist_repo::waive_checklist_item(
        &app_state.pool,
        &request.completed_by,
        &request.waive_reason,
        item_id,
        period_id,
        &tenant_id,
    )
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "Database error");
        with_request_id(ApiError::internal("Internal database error"), &ctx)
    })?
    .ok_or_else(|| {
        with_request_id(
            ApiError::not_found(format!("Checklist item {} not found", item_id)),
            &ctx,
        )
    })?;

    Ok(Json(to_checklist_response(row)))
}

/// GET /api/gl/periods/{period_id}/checklist
#[utoipa::path(get, path = "/api/gl/periods/{period_id}/checklist", tag = "Close Checklist",
    params(("period_id" = Uuid, Path, description = "Accounting period ID")),
    responses((status = 200, description = "Checklist status", body = PaginatedResponse<ChecklistItemResponse>)),
    security(("bearer" = [])))]
pub async fn get_checklist_status(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path(period_id): Path<Uuid>,
) -> Result<Json<PaginatedResponse<ChecklistItemResponse>>, ApiError> {
    let tenant_id = extract_tenant(&claims).map_err(|e| with_request_id(e, &ctx))?;

    let rows = checklist_repo::list_checklist_items(&app_state.pool, &tenant_id, period_id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Database error");
            with_request_id(ApiError::internal("Internal database error"), &ctx)
        })?;

    let items: Vec<ChecklistItemResponse> = rows.into_iter().map(to_checklist_response).collect();
    let total = items.len() as i64;
    Ok(Json(PaginatedResponse::new(items, 1, total, total)))
}

fn to_checklist_response(row: checklist_repo::ChecklistItemRow) -> ChecklistItemResponse {
    ChecklistItemResponse {
        id: row.id,
        tenant_id: row.tenant_id,
        period_id: row.period_id,
        label: row.label,
        status: row.status,
        completed_by: row.completed_by,
        completed_at: row.completed_at,
        waive_reason: row.waive_reason,
    }
}

// ============================================================
// APPROVAL TYPES
// ============================================================

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CreateApprovalRequest {
    pub actor_id: String,
    pub approval_type: String,
    pub notes: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ApprovalResponse {
    pub id: Uuid,
    pub tenant_id: String,
    pub period_id: Uuid,
    pub actor_id: String,
    pub approval_type: String,
    pub notes: Option<String>,
    pub approved_at: DateTime<Utc>,
}

// ============================================================
// APPROVAL HANDLERS
// ============================================================

/// POST /api/gl/periods/{period_id}/approvals — record an approval signoff (idempotent)
#[utoipa::path(post, path = "/api/gl/periods/{period_id}/approvals", tag = "Close Checklist",
    params(("period_id" = Uuid, Path, description = "Accounting period ID")),
    request_body = CreateApprovalRequest,
    responses((status = 201, description = "Approval recorded", body = ApprovalResponse)),
    security(("bearer" = [])))]
pub async fn create_approval(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path(period_id): Path<Uuid>,
    Json(request): Json<CreateApprovalRequest>,
) -> Result<(StatusCode, Json<ApprovalResponse>), ApiError> {
    let tenant_id = extract_tenant(&claims).map_err(|e| with_request_id(e, &ctx))?;

    let row = checklist_repo::create_approval(
        &app_state.pool,
        &tenant_id,
        period_id,
        &request.actor_id,
        &request.approval_type,
        request.notes.as_deref(),
    )
    .await
    .map_err(|e| {
        with_request_id(
            ApiError::internal(format!("Failed to record approval: {}", e)),
            &ctx,
        )
    })?;

    Ok((
        StatusCode::CREATED,
        Json(ApprovalResponse {
            id: row.id,
            tenant_id: row.tenant_id,
            period_id: row.period_id,
            actor_id: row.actor_id,
            approval_type: row.approval_type,
            notes: row.notes,
            approved_at: row.approved_at,
        }),
    ))
}

/// GET /api/gl/periods/{period_id}/approvals
#[utoipa::path(get, path = "/api/gl/periods/{period_id}/approvals", tag = "Close Checklist",
    params(("period_id" = Uuid, Path, description = "Accounting period ID")),
    responses((status = 200, description = "Approval list", body = PaginatedResponse<ApprovalResponse>)),
    security(("bearer" = [])))]
pub async fn get_approvals(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path(period_id): Path<Uuid>,
) -> Result<Json<PaginatedResponse<ApprovalResponse>>, ApiError> {
    let tenant_id = extract_tenant(&claims).map_err(|e| with_request_id(e, &ctx))?;

    let rows = checklist_repo::list_approvals(&app_state.pool, &tenant_id, period_id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Database error");
            with_request_id(ApiError::internal("Internal database error"), &ctx)
        })?;

    let items: Vec<ApprovalResponse> = rows
        .into_iter()
        .map(|r| ApprovalResponse {
            id: r.id,
            tenant_id: r.tenant_id,
            period_id: r.period_id,
            actor_id: r.actor_id,
            approval_type: r.approval_type,
            notes: r.notes,
            approved_at: r.approved_at,
        })
        .collect();
    let total = items.len() as i64;
    Ok(Json(PaginatedResponse::new(items, 1, total, total)))
}
