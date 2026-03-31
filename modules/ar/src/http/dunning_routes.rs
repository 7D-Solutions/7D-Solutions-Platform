use axum::{extract::State, Json};
use sqlx::PgPool;

use crate::models::ApiError;

// ============================================================================
// Dunning Scheduler (bd-2bj)
// ============================================================================

/// Request body for POST /api/ar/dunning/poll
#[derive(Debug, serde::Deserialize)]
pub struct DunningPollRequest {
    /// Maximum number of rows to process in this poll (default 10)
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,
}

fn default_batch_size() -> usize {
    10
}

/// Response body for POST /api/ar/dunning/poll
#[derive(Debug, serde::Serialize)]
pub struct DunningPollResponse {
    pub processed: usize,
    pub outcomes: Vec<crate::dunning_scheduler::DunningExecutionOutcome>,
}

/// POST /api/ar/dunning/poll — poll and execute due dunning rows
///
/// Claims due rows using FOR UPDATE SKIP LOCKED and executes the next
/// dunning action for each. Safe for concurrent workers — each row is
/// claimed exclusively.
pub async fn dunning_poll_route(
    State(db): State<PgPool>,
    Json(req): Json<DunningPollRequest>,
) -> Result<Json<DunningPollResponse>, ApiError> {
    let batch_size = req.batch_size.min(100); // Cap at 100
    let correlation_id = uuid::Uuid::new_v4().to_string();

    let outcomes = crate::dunning_scheduler::poll_and_execute_batch(
        &db,
        batch_size,
        &correlation_id,
        None, // No tenant filter — global scheduler
    )
    .await;

    let processed = outcomes
        .iter()
        .filter(|o| {
            matches!(
                o,
                crate::dunning_scheduler::DunningExecutionOutcome::Transitioned { .. }
            )
        })
        .count();

    Ok(Json(DunningPollResponse {
        processed,
        outcomes,
    }))
}
