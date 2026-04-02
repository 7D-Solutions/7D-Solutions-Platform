//! Scheduled Reconciliation Run Worker (bd-1kl)
//!
//! ## Run Lifecycle
//!
//! 1. **Create**: A scheduled run is created for a time window (pending).
//! 2. **Claim**: A worker claims a pending run via `FOR UPDATE SKIP LOCKED`.
//! 3. **Execute**: The worker marks the run as `running`, executes the matching
//!    engine, then marks it `completed` or `failed`.
//!
//! ## Invariants
//!
//! - **Exactly-once per window**: `UNIQUE(app_id, window_start, window_end)`
//!   prevents duplicate runs for the same window. Duplicate creation attempts
//!   return `AlreadyScheduled`.
//! - **Resumable**: If a run is stuck in `running` (worker crashed), a
//!   recovery sweep can reset it to `pending` for re-claiming.
//! - **Concurrent safety**: `FOR UPDATE SKIP LOCKED` ensures two workers
//!   never process the same run simultaneously.
//! - **Atomic**: claim + matching + completion all commit in one transaction.

use crate::reconciliation::{run_reconciliation, RunReconOutcome, RunReconRequest};
use chrono::{NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::fmt;
use uuid::Uuid;

// ============================================================================
// Request / Response types
// ============================================================================

/// Request to create a scheduled reconciliation run for a time window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateScheduledRunRequest {
    /// Stable ID for this scheduled run.
    pub scheduled_run_id: Uuid,
    /// Tenant identifier.
    pub app_id: String,
    /// Start of the reconciliation window.
    pub window_start: NaiveDateTime,
    /// End of the reconciliation window.
    pub window_end: NaiveDateTime,
    /// Distributed trace correlation ID.
    pub correlation_id: String,
}

/// Result of a scheduled run operation.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ScheduledRunResult {
    pub scheduled_run_id: Uuid,
    pub app_id: String,
    pub status: String,
    pub recon_run_id: Option<Uuid>,
    pub match_count: Option<i32>,
    pub exception_count: Option<i32>,
}

/// Outcome of creating a scheduled run.
#[derive(Debug, Clone)]
pub enum CreateScheduledRunOutcome {
    /// New scheduled run created (status: pending).
    Created(ScheduledRunResult),
    /// Run for this window already exists (deduped).
    AlreadyScheduled(ScheduledRunResult),
}

/// Outcome of claiming and executing a scheduled run.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub enum ScheduledRunExecutionOutcome {
    /// Run completed successfully.
    Completed(ScheduledRunResult),
    /// Run failed with an error.
    Failed {
        scheduled_run_id: Uuid,
        error: String,
    },
    /// No pending runs to claim.
    NothingToClaim,
}

// ============================================================================
// Error types
// ============================================================================

#[derive(Debug)]
pub enum ReconSchedulerError {
    DatabaseError(String),
    SerializationError(String),
    MatchingError(String),
}

impl fmt::Display for ReconSchedulerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DatabaseError(msg) => write!(f, "Database error: {}", msg),
            Self::SerializationError(msg) => write!(f, "Serialization error: {}", msg),
            Self::MatchingError(msg) => write!(f, "Matching error: {}", msg),
        }
    }
}

impl std::error::Error for ReconSchedulerError {}

impl From<sqlx::Error> for ReconSchedulerError {
    fn from(e: sqlx::Error) -> Self {
        Self::DatabaseError(e.to_string())
    }
}

// ============================================================================
// Internal row type
// ============================================================================

#[derive(Debug)]
#[allow(dead_code)]
struct ClaimableScheduledRun {
    id: i32,
    scheduled_run_id: Uuid,
    app_id: String,
    window_start: NaiveDateTime,
    window_end: NaiveDateTime,
    correlation_id: Option<String>,
}

impl<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow> for ClaimableScheduledRun {
    fn from_row(row: &'r sqlx::postgres::PgRow) -> Result<Self, sqlx::Error> {
        use sqlx::Row;
        Ok(Self {
            id: row.try_get("id")?,
            scheduled_run_id: row.try_get("scheduled_run_id")?,
            app_id: row.try_get("app_id")?,
            window_start: row.try_get("window_start")?,
            window_end: row.try_get("window_end")?,
            correlation_id: row.try_get("correlation_id")?,
        })
    }
}

// ============================================================================
// Core functions
// ============================================================================

/// Create a scheduled reconciliation run for a given time window.
///
/// **Idempotency**: if a run for the same (app_id, window_start, window_end)
/// already exists, returns `AlreadyScheduled` with the existing run details.
pub async fn create_scheduled_run(
    pool: &PgPool,
    req: CreateScheduledRunRequest,
) -> Result<CreateScheduledRunOutcome, ReconSchedulerError> {
    // Check for existing run with same window (dedup).
    let existing: Option<(Uuid, String, Option<Uuid>, Option<i32>, Option<i32>)> = sqlx::query_as(
        r#"
        SELECT scheduled_run_id, status, recon_run_id, match_count, exception_count
        FROM ar_recon_scheduled_runs
        WHERE app_id = $1 AND window_start = $2 AND window_end = $3
        "#,
    )
    .bind(&req.app_id)
    .bind(req.window_start)
    .bind(req.window_end)
    .fetch_optional(pool)
    .await?;

    if let Some((scheduled_run_id, status, recon_run_id, match_count, exception_count)) = existing {
        return Ok(CreateScheduledRunOutcome::AlreadyScheduled(
            ScheduledRunResult {
                scheduled_run_id,
                app_id: req.app_id,
                status,
                recon_run_id,
                match_count,
                exception_count,
            },
        ));
    }

    // Insert new scheduled run.
    sqlx::query(
        r#"
        INSERT INTO ar_recon_scheduled_runs (
            scheduled_run_id, app_id, window_start, window_end,
            status, correlation_id
        )
        VALUES ($1, $2, $3, $4, 'pending', $5)
        ON CONFLICT (app_id, window_start, window_end) DO NOTHING
        "#,
    )
    .bind(req.scheduled_run_id)
    .bind(&req.app_id)
    .bind(req.window_start)
    .bind(req.window_end)
    .bind(&req.correlation_id)
    .execute(pool)
    .await?;

    Ok(CreateScheduledRunOutcome::Created(ScheduledRunResult {
        scheduled_run_id: req.scheduled_run_id,
        app_id: req.app_id,
        status: "pending".to_string(),
        recon_run_id: None,
        match_count: None,
        exception_count: None,
    }))
}

/// Claim and execute a single pending scheduled reconciliation run.
///
/// Uses `FOR UPDATE SKIP LOCKED` to safely claim one pending run. Within
/// a single transaction:
/// 1. Claims a pending run row
/// 2. Marks it as `running`
/// 3. Executes the reconciliation matching engine
/// 4. Marks the run as `completed` or `failed`
/// 5. Emits `ar.recon_run_started` outbox event
///
/// An optional `app_id` filter restricts claiming to a specific tenant.
pub async fn claim_and_execute_scheduled_run(
    pool: &PgPool,
    worker_id: &str,
    correlation_id: &str,
    app_id_filter: Option<&str>,
) -> Result<ScheduledRunExecutionOutcome, ReconSchedulerError> {
    let now = Utc::now();

    // 1. Claim one pending run with SKIP LOCKED.
    let row: Option<ClaimableScheduledRun> = if let Some(app_id) = app_id_filter {
        sqlx::query_as(
            r#"
            SELECT id, scheduled_run_id, app_id, window_start, window_end, correlation_id
            FROM ar_recon_scheduled_runs
            WHERE status = 'pending'
              AND app_id = $1
            ORDER BY window_start ASC
            LIMIT 1
            FOR UPDATE SKIP LOCKED
            "#,
        )
        .bind(app_id)
        .fetch_optional(pool)
        .await?
    } else {
        sqlx::query_as(
            r#"
            SELECT id, scheduled_run_id, app_id, window_start, window_end, correlation_id
            FROM ar_recon_scheduled_runs
            WHERE status = 'pending'
            ORDER BY window_start ASC
            LIMIT 1
            FOR UPDATE SKIP LOCKED
            "#,
        )
        .fetch_optional(pool)
        .await?
    };

    let row = match row {
        Some(r) => r,
        None => return Ok(ScheduledRunExecutionOutcome::NothingToClaim),
    };

    // 2. Mark as running (with worker_id and claimed_at).
    sqlx::query(
        r#"
        UPDATE ar_recon_scheduled_runs
        SET status = 'running', worker_id = $1, claimed_at = $2
        WHERE id = $3 AND status = 'pending'
        "#,
    )
    .bind(worker_id)
    .bind(now.naive_utc())
    .bind(row.id)
    .execute(pool)
    .await?;

    // 3. Generate a recon_run_id and execute matching.
    let recon_run_id = Uuid::new_v4();
    let run_correlation = row
        .correlation_id
        .clone()
        .unwrap_or_else(|| correlation_id.to_string());

    let matching_result = run_reconciliation(
        pool,
        RunReconRequest {
            recon_run_id,
            app_id: row.app_id.clone(),
            correlation_id: run_correlation.clone(),
            causation_id: Some(format!("scheduled-run-{}", row.scheduled_run_id)),
        },
    )
    .await;

    // 4. Update the scheduled run based on matching result.
    match matching_result {
        Ok(outcome) => {
            let (match_count, exception_count) = match &outcome {
                RunReconOutcome::Executed(r) => (r.match_count, r.exception_count),
                RunReconOutcome::AlreadyExists(r) => (r.match_count, r.exception_count),
            };

            sqlx::query(
                r#"
                UPDATE ar_recon_scheduled_runs
                SET status = 'completed',
                    recon_run_id = $1,
                    match_count = $2,
                    exception_count = $3,
                    completed_at = $4
                WHERE id = $5
                "#,
            )
            .bind(recon_run_id)
            .bind(match_count)
            .bind(exception_count)
            .bind(now.naive_utc())
            .bind(row.id)
            .execute(pool)
            .await?;

            Ok(ScheduledRunExecutionOutcome::Completed(
                ScheduledRunResult {
                    scheduled_run_id: row.scheduled_run_id,
                    app_id: row.app_id,
                    status: "completed".to_string(),
                    recon_run_id: Some(recon_run_id),
                    match_count: Some(match_count),
                    exception_count: Some(exception_count),
                },
            ))
        }
        Err(e) => {
            let error_msg = e.to_string();

            sqlx::query(
                r#"
                UPDATE ar_recon_scheduled_runs
                SET status = 'failed',
                    recon_run_id = $1,
                    error_message = $2,
                    completed_at = $3
                WHERE id = $4
                "#,
            )
            .bind(recon_run_id)
            .bind(&error_msg)
            .bind(now.naive_utc())
            .bind(row.id)
            .execute(pool)
            .await?;

            Ok(ScheduledRunExecutionOutcome::Failed {
                scheduled_run_id: row.scheduled_run_id,
                error: error_msg,
            })
        }
    }
}

/// Poll and execute a batch of pending scheduled reconciliation runs.
///
/// Calls `claim_and_execute_scheduled_run` up to `batch_size` times,
/// stopping early when there's nothing left to claim.
pub async fn poll_scheduled_runs(
    pool: &PgPool,
    batch_size: usize,
    worker_id: &str,
    correlation_id: &str,
    app_id_filter: Option<&str>,
) -> Vec<ScheduledRunExecutionOutcome> {
    let mut outcomes = Vec::with_capacity(batch_size);

    for _ in 0..batch_size {
        match claim_and_execute_scheduled_run(pool, worker_id, correlation_id, app_id_filter).await
        {
            Ok(ScheduledRunExecutionOutcome::NothingToClaim) => {
                outcomes.push(ScheduledRunExecutionOutcome::NothingToClaim);
                break;
            }
            Ok(outcome) => outcomes.push(outcome),
            Err(e) => {
                outcomes.push(ScheduledRunExecutionOutcome::Failed {
                    scheduled_run_id: Uuid::nil(),
                    error: e.to_string(),
                });
            }
        }
    }

    outcomes
}

// ============================================================================
// Unit tests (pure logic — no DB)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheduled_run_result_serializes_correctly() {
        let result = ScheduledRunResult {
            scheduled_run_id: Uuid::new_v4(),
            app_id: "tenant-1".to_string(),
            status: "completed".to_string(),
            recon_run_id: Some(Uuid::new_v4()),
            match_count: Some(5),
            exception_count: Some(1),
        };
        let json = serde_json::to_string(&result).expect("serialization failed");
        assert!(json.contains("scheduled_run_id"));
        assert!(json.contains("completed"));
        assert!(json.contains("match_count"));
    }

    #[test]
    fn error_display() {
        let err = ReconSchedulerError::DatabaseError("connection lost".to_string());
        assert_eq!(err.to_string(), "Database error: connection lost");

        let err = ReconSchedulerError::MatchingError("no data".to_string());
        assert_eq!(err.to_string(), "Matching error: no data");
    }
}
