//! Bill run HTTP handler.
//!
//! POST /api/bill-runs/execute — Execute billing cycle.
//! Delegates to [`super::bill_run_service`] for business logic.

use axum::{extract::State, http::StatusCode, Extension, Json};
use chrono::Utc;
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::{BillRunResult, ExecuteBillRunRequest};
use platform_sdk::extract_tenant;

use super::bill_run_service;

fn with_request_id(err: ApiError, ctx: &Option<Extension<TracingContext>>) -> ApiError {
    match ctx {
        Some(Extension(c)) => {
            if let Some(tid) = &c.trace_id {
                err.with_request_id(tid.clone())
            } else {
                err
            }
        }
        None => err,
    }
}

/// POST /api/bill-runs/execute - Execute billing cycle
#[utoipa::path(
    post, path = "/api/bill-runs/execute", tag = "Bill Runs",
    request_body = ExecuteBillRunRequest,
    responses(
        (status = 200, description = "Bill run completed (or idempotent replay)", body = BillRunResult),
        (status = 401, body = ApiError), (status = 500, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn execute_bill_run(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(req): Json<ExecuteBillRunRequest>,
) -> Result<(StatusCode, Json<BillRunResult>), ApiError> {
    let raw_ctx = tracing_ctx
        .as_ref()
        .map(|Extension(c)| c.clone())
        .unwrap_or_default();
    let tenant_id =
        extract_tenant(&claims).map_err(|e| with_request_id(e, &tracing_ctx))?;

    let bill_run_id = req
        .bill_run_id
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let execution_date = req
        .execution_date
        .unwrap_or_else(|| Utc::now().date_naive());

    let result = bill_run_service::execute_bill_run(
        &db,
        &tenant_id,
        &bill_run_id,
        execution_date,
        &raw_ctx,
    )
    .await
    .map_err(|e| with_request_id(e, &tracing_ctx))?;

    Ok((StatusCode::OK, Json(result)))
}
