use axum::{
    extract::State,
    http::StatusCode,
    Json,
};
use sqlx::PgPool;

use crate::models::ErrorResponse;

// ============================================================================
// Reconciliation Matching (bd-2cn)
// ============================================================================

/// Request body for POST /api/ar/recon/run
#[derive(Debug, serde::Deserialize)]
pub struct ReconRunRequest {
    /// Stable ID for this reconciliation run (idempotency anchor).
    pub recon_run_id: Option<uuid::Uuid>,
}

/// POST /api/ar/recon/run — trigger a reconciliation matching run
///
/// Matches unmatched succeeded payments against open invoices using
/// deterministic heuristic rules. Same inputs always produce same outputs.
pub async fn recon_run_route(
    State(db): State<PgPool>,
    Json(req): Json<ReconRunRequest>,
) -> Result<Json<crate::reconciliation::ReconRunResult>, (StatusCode, Json<ErrorResponse>)> {
    let recon_run_id = req.recon_run_id.unwrap_or_else(uuid::Uuid::new_v4);
    let app_id = "test-app".to_string(); // TODO: extract from auth middleware

    let result = crate::reconciliation::run_reconciliation(
        &db,
        crate::reconciliation::RunReconRequest {
            recon_run_id,
            app_id,
            correlation_id: uuid::Uuid::new_v4().to_string(),
            causation_id: None,
        },
    )
    .await
    .map_err(|e| {
        tracing::error!("Reconciliation run failed: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new("recon_error", e.to_string())),
        )
    })?;

    match result {
        crate::reconciliation::RunReconOutcome::Executed(r) => Ok(Json(r)),
        crate::reconciliation::RunReconOutcome::AlreadyExists(r) => Ok(Json(r)),
    }
}

// ============================================================================
// Scheduled Reconciliation Runs (bd-1kl)
// ============================================================================

/// Request body for POST /api/ar/recon/schedule
#[derive(Debug, serde::Deserialize)]
pub struct ScheduleReconRequest {
    pub scheduled_run_id: Option<uuid::Uuid>,
    pub app_id: String,
    pub window_start: chrono::NaiveDateTime,
    pub window_end: chrono::NaiveDateTime,
}

/// POST /api/ar/recon/schedule — create a scheduled reconciliation run
pub async fn schedule_recon_route(
    State(db): State<PgPool>,
    Json(req): Json<ScheduleReconRequest>,
) -> Result<Json<crate::recon_scheduler::ScheduledRunResult>, (StatusCode, Json<ErrorResponse>)> {
    let scheduled_run_id = req.scheduled_run_id.unwrap_or_else(uuid::Uuid::new_v4);

    let result = crate::recon_scheduler::create_scheduled_run(
        &db,
        crate::recon_scheduler::CreateScheduledRunRequest {
            scheduled_run_id,
            app_id: req.app_id,
            window_start: req.window_start,
            window_end: req.window_end,
            correlation_id: uuid::Uuid::new_v4().to_string(),
        },
    )
    .await
    .map_err(|e| {
        tracing::error!("Schedule recon run failed: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new("recon_schedule_error", e.to_string())),
        )
    })?;

    match result {
        crate::recon_scheduler::CreateScheduledRunOutcome::Created(r) => Ok(Json(r)),
        crate::recon_scheduler::CreateScheduledRunOutcome::AlreadyScheduled(r) => Ok(Json(r)),
    }
}

/// Request body for POST /api/ar/recon/poll
#[derive(Debug, serde::Deserialize)]
pub struct ReconPollRequest {
    pub worker_id: Option<String>,
    pub app_id: Option<String>,
    pub batch_size: Option<usize>,
}

/// POST /api/ar/recon/poll — claim and execute pending scheduled runs
pub async fn recon_poll_route(
    State(db): State<PgPool>,
    Json(req): Json<ReconPollRequest>,
) -> Result<Json<Vec<crate::recon_scheduler::ScheduledRunExecutionOutcome>>, (StatusCode, Json<ErrorResponse>)> {
    let worker_id = req.worker_id.unwrap_or_else(|| "api-worker".to_string());
    let batch_size = req.batch_size.unwrap_or(10);
    let correlation_id = uuid::Uuid::new_v4().to_string();

    let outcomes = crate::recon_scheduler::poll_scheduled_runs(
        &db,
        batch_size,
        &worker_id,
        &correlation_id,
        req.app_id.as_deref(),
    )
    .await;

    Ok(Json(outcomes))
}
