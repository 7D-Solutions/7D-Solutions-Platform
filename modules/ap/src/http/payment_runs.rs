//! HTTP handlers for AP payment run endpoints.
//!
//! POST /api/ap/payment-runs       — create a payment run (idempotent via run_id)
//! GET  /api/ap/payment-runs/:id   — get a payment run with its items

use axum::{
    extract::{Path, State},
    response::IntoResponse,
    Extension, Json,
};
use chrono::{DateTime, Utc};
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::payment_runs::{
    builder::create_payment_run, execute::execute_payment_run, CreatePaymentRunRequest,
};
use platform_sdk::extract_tenant;
use crate::http::tenant::with_request_id;
use crate::AppState;

// ============================================================================
// Request body
// ============================================================================

#[derive(Debug, Deserialize, utoipa::ToSchema)]
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
// Handlers
// ============================================================================

/// POST /api/ap/payment-runs
///
/// Create a payment run by selecting all eligible bills for the tenant.
/// Idempotent: supplying the same `run_id` returns the existing run (200 OK).
#[utoipa::path(post, path = "/api/ap/payment-runs", tag = "Payment Runs",
    request_body = CreatePaymentRunBody,
    responses((status = 201, description = "Run created", body = serde_json::Value)), security(("bearer" = [])))]
pub async fn create_run(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(body): Json<CreatePaymentRunBody>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

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

    match create_payment_run(&state.pool, &tenant_id, &req).await {
        Ok(result) => {
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

            Json(json!({
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
            }))
            .into_response()
        }
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

/// GET /api/ap/payment-runs/:run_id
///
/// Fetch a payment run and its items.
#[utoipa::path(get, path = "/api/ap/payment-runs/{run_id}", tag = "Payment Runs",
    params(("run_id" = Uuid, Path)), responses((status = 200, description = "Run details", body = serde_json::Value)),
    security(("bearer" = [])))]
pub async fn get_run(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(run_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    let run: Option<crate::domain::payment_runs::PaymentRun> = match sqlx::query_as(
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
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "DB error fetching payment run");
            return with_request_id(ApiError::internal("An internal error occurred"), &tracing_ctx)
                .into_response();
        }
    };

    let run = match run {
        Some(r) => r,
        None => {
            return with_request_id(
                ApiError::not_found(format!("Payment run {} not found", run_id)),
                &tracing_ctx,
            )
            .into_response()
        }
    };

    let items: Vec<crate::domain::payment_runs::PaymentRunItemRow> = match sqlx::query_as(
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
    {
        Ok(items) => items,
        Err(e) => {
            tracing::error!(error = %e, "DB error fetching payment run items");
            return with_request_id(ApiError::internal("An internal error occurred"), &tracing_ctx)
                .into_response();
        }
    };

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

    Json(json!({
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
    }))
    .into_response()
}

/// POST /api/ap/payment-runs/:run_id/execute
///
/// Execute a payment run: submit payments to the disbursement layer,
/// record allocations, mark bills paid, and emit `ap.payment_executed` events.
///
/// Idempotent: calling this endpoint on an already-completed run returns the
/// existing execution state with 200 OK.
#[utoipa::path(post, path = "/api/ap/payment-runs/{run_id}/execute", tag = "Payment Runs",
    params(("run_id" = Uuid, Path)), responses((status = 200, description = "Run executed", body = serde_json::Value)),
    security(("bearer" = [])))]
pub async fn execute_run(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(run_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match execute_payment_run(&state.pool, &tenant_id, run_id).await {
        Ok(result) => {
            let executions_json: Vec<serde_json::Value> = result
                .executions
                .iter()
                .map(|e| {
                    json!({
                        "id": e.id,
                        "item_id": e.item_id,
                        "payment_id": e.payment_id,
                        "vendor_id": e.vendor_id,
                        "amount_minor": e.amount_minor,
                        "currency": e.currency,
                        "status": e.status,
                        "executed_at": e.executed_at,
                    })
                })
                .collect();

            Json(json!({
                "run_id": result.run.run_id,
                "status": result.run.status,
                "executed_at": result.run.executed_at,
                "executions": executions_json,
            }))
            .into_response()
        }
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}
