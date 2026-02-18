//! HTTP handlers for AP payment run endpoints.
//!
//! POST /api/ap/payment-runs       — create a payment run (idempotent via run_id)
//! GET  /api/ap/payment-runs/:id   — get a payment run with its items

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::payment_runs::{
    builder::create_payment_run, CreatePaymentRunRequest, PaymentRunError,
};
use crate::http::vendors::ErrorBody;
use crate::AppState;

// ============================================================================
// Request body
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct CreatePaymentRunBody {
    pub run_id: Option<Uuid>,
    pub currency: String,
    pub scheduled_date: DateTime<Utc>,
    pub payment_method: String,
    pub created_by: String,
    pub due_on_or_before: Option<DateTime<Utc>>,
    pub vendor_ids: Option<Vec<Uuid>>,
}

// ============================================================================
// Shared helpers
// ============================================================================

fn tenant_from_headers(headers: &HeaderMap) -> Result<String, (StatusCode, Json<ErrorBody>)> {
    headers
        .get("x-tenant-id")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string())
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorBody::new("missing_tenant", "X-Tenant-Id header is required")),
            )
        })
}

fn run_error_response(e: PaymentRunError) -> (StatusCode, Json<ErrorBody>) {
    match e {
        PaymentRunError::NoBillsEligible(tenant, currency) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorBody::new(
                "no_eligible_bills",
                &format!(
                    "No eligible bills found for tenant '{}' in currency '{}'",
                    tenant, currency
                ),
            )),
        ),
        PaymentRunError::DuplicateRunId(id) => (
            StatusCode::CONFLICT,
            Json(ErrorBody::new(
                "duplicate_run_id",
                &format!("Payment run {} already exists for a different tenant", id),
            )),
        ),
        PaymentRunError::Validation(msg) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorBody::new("validation_error", &msg)),
        ),
        PaymentRunError::Database(e) => {
            tracing::error!(error = %e, "Database error in payment run handler");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody::new("database_error", "An internal error occurred")),
            )
        }
    }
}

// ============================================================================
// Handlers
// ============================================================================

/// POST /api/ap/payment-runs
///
/// Create a payment run by selecting all eligible bills for the tenant.
/// Idempotent: supplying the same `run_id` returns the existing run (200 OK).
pub async fn create_run(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<CreatePaymentRunBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorBody>)> {
    let tenant_id = tenant_from_headers(&headers)?;

    let run_id = body.run_id.unwrap_or_else(Uuid::new_v4);

    let req = CreatePaymentRunRequest {
        run_id,
        currency: body.currency,
        scheduled_date: body.scheduled_date,
        payment_method: body.payment_method,
        created_by: body.created_by,
        due_on_or_before: body.due_on_or_before,
        vendor_ids: body.vendor_ids,
        correlation_id: None,
    };

    let result = create_payment_run(&state.pool, &tenant_id, &req)
        .await
        .map_err(run_error_response)?;

    let items: Vec<serde_json::Value> = result
        .items
        .iter()
        .map(|item| {
            json!({
                "id": item.id,
                "vendor_id": item.vendor_id,
                "bill_ids": item.bill_ids,
                "amount_minor": item.amount_minor,
                "currency": item.currency,
            })
        })
        .collect();

    Ok(Json(json!({
        "run_id": result.run.run_id,
        "tenant_id": result.run.tenant_id,
        "status": result.run.status,
        "total_minor": result.run.total_minor,
        "currency": result.run.currency,
        "scheduled_date": result.run.scheduled_date,
        "payment_method": result.run.payment_method,
        "created_by": result.run.created_by,
        "created_at": result.run.created_at,
        "items": items,
    })))
}

/// GET /api/ap/payment-runs/:run_id
///
/// Fetch a payment run and its items.
pub async fn get_run(
    State(state): State<Arc<AppState>>,
    Path(run_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ErrorBody>)> {
    let tenant_id = tenant_from_headers(&headers)?;

    let run: Option<crate::domain::payment_runs::PaymentRun> = sqlx::query_as(
        r#"
        SELECT run_id, tenant_id, total_minor, currency, scheduled_date,
               payment_method, status, created_by, created_at, executed_at
        FROM payment_runs
        WHERE run_id = $1 AND tenant_id = $2
        "#,
    )
    .bind(run_id)
    .bind(&tenant_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "DB error fetching payment run");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorBody::new("database_error", "An internal error occurred")),
        )
    })?;

    let run = run.ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorBody::new(
                "not_found",
                &format!("Payment run {} not found", run_id),
            )),
        )
    })?;

    let items: Vec<crate::domain::payment_runs::PaymentRunItemRow> = sqlx::query_as(
        r#"
        SELECT id, run_id, vendor_id, bill_ids, amount_minor, currency, created_at
        FROM payment_run_items
        WHERE run_id = $1
        ORDER BY id ASC
        "#,
    )
    .bind(run_id)
    .fetch_all(&state.pool)
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "DB error fetching payment run items");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorBody::new("database_error", "An internal error occurred")),
        )
    })?;

    let items_json: Vec<serde_json::Value> = items
        .iter()
        .map(|item| {
            json!({
                "id": item.id,
                "vendor_id": item.vendor_id,
                "bill_ids": item.bill_ids,
                "amount_minor": item.amount_minor,
                "currency": item.currency,
            })
        })
        .collect();

    Ok(Json(json!({
        "run_id": run.run_id,
        "tenant_id": run.tenant_id,
        "status": run.status,
        "total_minor": run.total_minor,
        "currency": run.currency,
        "scheduled_date": run.scheduled_date,
        "payment_method": run.payment_method,
        "created_by": run.created_by,
        "created_at": run.created_at,
        "executed_at": run.executed_at,
        "items": items_json,
    })))
}
