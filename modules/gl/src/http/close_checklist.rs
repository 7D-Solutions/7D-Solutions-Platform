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
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

use super::auth::{extract_tenant, with_request_id};
use crate::AppState;

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

#[derive(Debug, sqlx::FromRow)]
struct ChecklistItemRow {
    id: Uuid,
    tenant_id: String,
    period_id: Uuid,
    label: String,
    status: String,
    completed_by: Option<String>,
    completed_at: Option<DateTime<Utc>>,
    waive_reason: Option<String>,
}

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

    let row = sqlx::query_as::<_, ChecklistItemRow>(
        r#"
        INSERT INTO close_checklist_items (tenant_id, period_id, label)
        VALUES ($1, $2, $3)
        RETURNING id, tenant_id, period_id, label, status, completed_by, completed_at, waive_reason
        "#,
    )
    .bind(&tenant_id)
    .bind(period_id)
    .bind(&request.label)
    .fetch_one(&app_state.pool)
    .await
    .map_err(|e| with_request_id(ApiError::internal(format!("Failed to create checklist item: {}", e)), &ctx))?;

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

    let row = sqlx::query_as::<_, ChecklistItemRow>(
        r#"
        UPDATE close_checklist_items
        SET status = 'complete', completed_by = $1, completed_at = NOW(), updated_at = NOW()
        WHERE id = $2 AND period_id = $3 AND tenant_id = $4
        RETURNING id, tenant_id, period_id, label, status, completed_by, completed_at, waive_reason
        "#,
    )
    .bind(&request.completed_by)
    .bind(item_id)
    .bind(period_id)
    .bind(&tenant_id)
    .fetch_optional(&app_state.pool)
    .await
    .map_err(|e| {
        tracing::error!("Database error: {}", e);
        with_request_id(ApiError::internal("Internal database error"), &ctx)
    })?
    .ok_or_else(|| {
        with_request_id(ApiError::not_found(format!("Checklist item {} not found", item_id)), &ctx)
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

    let row = sqlx::query_as::<_, ChecklistItemRow>(
        r#"
        UPDATE close_checklist_items
        SET status = 'waived', completed_by = $1, completed_at = NOW(),
            waive_reason = $2, updated_at = NOW()
        WHERE id = $3 AND period_id = $4 AND tenant_id = $5
        RETURNING id, tenant_id, period_id, label, status, completed_by, completed_at, waive_reason
        "#,
    )
    .bind(&request.completed_by)
    .bind(&request.waive_reason)
    .bind(item_id)
    .bind(period_id)
    .bind(&tenant_id)
    .fetch_optional(&app_state.pool)
    .await
    .map_err(|e| {
        tracing::error!("Database error: {}", e);
        with_request_id(ApiError::internal("Internal database error"), &ctx)
    })?
    .ok_or_else(|| {
        with_request_id(ApiError::not_found(format!("Checklist item {} not found", item_id)), &ctx)
    })?;

    Ok(Json(to_checklist_response(row)))
}

/// GET /api/gl/periods/{period_id}/checklist
#[utoipa::path(get, path = "/api/gl/periods/{period_id}/checklist", tag = "Close Checklist",
    params(("period_id" = Uuid, Path, description = "Accounting period ID")),
    responses((status = 200, description = "Checklist status", body = Vec<ChecklistItemResponse>)),
    security(("bearer" = [])))]
pub async fn get_checklist_status(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path(period_id): Path<Uuid>,
) -> Result<Json<Vec<ChecklistItemResponse>>, ApiError> {
    let tenant_id = extract_tenant(&claims).map_err(|e| with_request_id(e, &ctx))?;

    let rows = sqlx::query_as::<_, ChecklistItemRow>(
        r#"
        SELECT id, tenant_id, period_id, label, status, completed_by, completed_at, waive_reason
        FROM close_checklist_items
        WHERE tenant_id = $1 AND period_id = $2
        ORDER BY created_at ASC
        "#,
    )
    .bind(&tenant_id)
    .bind(period_id)
    .fetch_all(&app_state.pool)
    .await
    .map_err(|e| {
        tracing::error!("Database error: {}", e);
        with_request_id(ApiError::internal("Internal database error"), &ctx)
    })?;

    Ok(Json(rows.into_iter().map(to_checklist_response).collect()))
}

fn to_checklist_response(row: ChecklistItemRow) -> ChecklistItemResponse {
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

#[derive(Debug, sqlx::FromRow)]
struct ApprovalRow {
    id: Uuid,
    tenant_id: String,
    period_id: Uuid,
    actor_id: String,
    approval_type: String,
    notes: Option<String>,
    approved_at: DateTime<Utc>,
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

    let row = sqlx::query_as::<_, ApprovalRow>(
        r#"
        INSERT INTO close_approvals (tenant_id, period_id, actor_id, approval_type, notes)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (tenant_id, period_id, approval_type) DO UPDATE
            SET actor_id = EXCLUDED.actor_id, notes = EXCLUDED.notes, approved_at = NOW()
        RETURNING id, tenant_id, period_id, actor_id, approval_type, notes, approved_at
        "#,
    )
    .bind(&tenant_id)
    .bind(period_id)
    .bind(&request.actor_id)
    .bind(&request.approval_type)
    .bind(&request.notes)
    .fetch_one(&app_state.pool)
    .await
    .map_err(|e| with_request_id(ApiError::internal(format!("Failed to record approval: {}", e)), &ctx))?;

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
    responses((status = 200, description = "Approval list", body = Vec<ApprovalResponse>)),
    security(("bearer" = [])))]
pub async fn get_approvals(
    State(app_state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path(period_id): Path<Uuid>,
) -> Result<Json<Vec<ApprovalResponse>>, ApiError> {
    let tenant_id = extract_tenant(&claims).map_err(|e| with_request_id(e, &ctx))?;

    let rows = sqlx::query_as::<_, ApprovalRow>(
        r#"
        SELECT id, tenant_id, period_id, actor_id, approval_type, notes, approved_at
        FROM close_approvals
        WHERE tenant_id = $1 AND period_id = $2
        ORDER BY approved_at ASC
        "#,
    )
    .bind(&tenant_id)
    .bind(period_id)
    .fetch_all(&app_state.pool)
    .await
    .map_err(|e| {
        tracing::error!("Database error: {}", e);
        with_request_id(ApiError::internal("Internal database error"), &ctx)
    })?;

    Ok(Json(
        rows.into_iter()
            .map(|r| ApprovalResponse {
                id: r.id,
                tenant_id: r.tenant_id,
                period_id: r.period_id,
                actor_id: r.actor_id,
                approval_type: r.approval_type,
                notes: r.notes,
                approved_at: r.approved_at,
            })
            .collect(),
    ))
}
