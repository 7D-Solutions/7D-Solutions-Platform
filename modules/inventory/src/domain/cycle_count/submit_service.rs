//! Cycle count submit service.
//!
//! Submitting a count fills in counted_qty on each line, computes variance_qty
//! deterministically from the expected_qty snapshotted at task creation, and
//! moves the task from 'open' → 'submitted'. Stock adjustments are NOT applied
//! here — that happens on approve (bd-opin).
//!
//! Pattern: Guard → Mutation → Outbox (all in one transaction)
//!
//! Guards:
//!   - tenant_id must be non-empty
//!   - idempotency_key must be non-empty
//!   - task must exist and belong to this tenant
//!   - task must be in 'open' status
//!   - counted_qty must be >= 0 for every input line
//!   - each input line_id must belong to this task

use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

use crate::events::cycle_count_submitted::{
    build_cycle_count_submitted_envelope, CycleCountSubmittedLine, CycleCountSubmittedPayload,
    EVENT_TYPE_CYCLE_COUNT_SUBMITTED,
};

// ============================================================================
// Types
// ============================================================================

/// One line in the submit request (caller supplies counted_qty per line_id).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitLineInput {
    pub line_id: Uuid,
    /// Physical count; must be >= 0
    pub counted_qty: i64,
}

/// Full submit request (task_id from URL path, rest from request body).
#[derive(Debug, Serialize, Deserialize)]
pub struct SubmitRequest {
    pub task_id: Uuid,
    pub tenant_id: String,
    pub idempotency_key: String,
    #[serde(default)]
    pub lines: Vec<SubmitLineInput>,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
}

/// One submitted line in the result — includes variance computed by the service.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmittedLine {
    pub line_id: Uuid,
    pub item_id: Uuid,
    pub expected_qty: i64,
    pub counted_qty: i64,
    /// Computed: counted_qty − expected_qty
    pub variance_qty: i64,
}

/// Returned on successful or replayed submit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitResult {
    pub task_id: Uuid,
    pub tenant_id: String,
    pub status: String,
    pub submitted_at: chrono::DateTime<Utc>,
    pub line_count: usize,
    pub lines: Vec<SubmittedLine>,
}

#[derive(Debug, Error)]
pub enum SubmitError {
    #[error("tenant_id is required")]
    MissingTenant,

    #[error("idempotency_key is required")]
    MissingIdempotencyKey,

    #[error("task not found or does not belong to this tenant")]
    TaskNotFound,

    #[error("task is not open (current status: {current_status})")]
    TaskNotOpen { current_status: String },

    #[error("line {line_id} not found in this task")]
    LineNotFound { line_id: Uuid },

    #[error("counted_qty must be >= 0 for line {line_id}")]
    NegativeCountedQty { line_id: Uuid },

    #[error("idempotency key conflict: same key used with a different request body")]
    ConflictingIdempotencyKey,

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ============================================================================
// Internal row types
// ============================================================================

#[derive(sqlx::FromRow)]
struct TaskRow {
    status: String,
    warehouse_id: Uuid,
    location_id: Uuid,
}

#[derive(sqlx::FromRow)]
struct LineRow {
    id: Uuid,
    item_id: Uuid,
    expected_qty: i64,
}

#[derive(sqlx::FromRow)]
struct IdempotencyRecord {
    response_body: String,
    request_hash: String,
}

// ============================================================================
// Service
// ============================================================================

/// Submit a cycle count task.
///
/// Returns `(SubmitResult, is_replay)`.
/// - `is_replay = false`: task moved to submitted; HTTP 201.
/// - `is_replay = true`:  idempotency key matched; HTTP 200 with stored result.
pub async fn submit_cycle_count(
    pool: &PgPool,
    req: &SubmitRequest,
) -> Result<(SubmitResult, bool), SubmitError> {
    // --- Stateless validation ---
    validate_request(req)?;

    // --- Idempotency check (fast path for replays) ---
    let request_hash = serde_json::to_string(req)?;
    if let Some(record) = find_idempotency_key(pool, &req.tenant_id, &req.idempotency_key).await? {
        if record.request_hash != request_hash {
            return Err(SubmitError::ConflictingIdempotencyKey);
        }
        let result: SubmitResult = serde_json::from_str(&record.response_body)?;
        return Ok((result, true));
    }

    // --- Guard: load task, verify tenant, check status ---
    let task: Option<TaskRow> = sqlx::query_as::<_, TaskRow>(
        r#"
        SELECT status::TEXT AS status, warehouse_id, location_id
        FROM cycle_count_tasks
        WHERE id = $1 AND tenant_id = $2
        "#,
    )
    .bind(req.task_id)
    .bind(&req.tenant_id)
    .fetch_optional(pool)
    .await?;

    let task = task.ok_or(SubmitError::TaskNotFound)?;

    if task.status != "open" {
        return Err(SubmitError::TaskNotOpen {
            current_status: task.status,
        });
    }

    // --- Guard: counted_qty >= 0 for all input lines ---
    for line_input in &req.lines {
        if line_input.counted_qty < 0 {
            return Err(SubmitError::NegativeCountedQty {
                line_id: line_input.line_id,
            });
        }
    }

    // --- Load all existing lines for this task ---
    let existing_lines: Vec<LineRow> = sqlx::query_as::<_, LineRow>(
        r#"
        SELECT id, item_id, expected_qty
        FROM cycle_count_lines
        WHERE task_id = $1
        "#,
    )
    .bind(req.task_id)
    .fetch_all(pool)
    .await?;

    // --- Guard: every input line_id must exist in this task ---
    for line_input in &req.lines {
        let exists = existing_lines.iter().any(|l| l.id == line_input.line_id);
        if !exists {
            return Err(SubmitError::LineNotFound {
                line_id: line_input.line_id,
            });
        }
    }

    let submitted_at = Utc::now();
    let event_id = Uuid::new_v4();
    let correlation_id = req
        .correlation_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let mut tx = pool.begin().await?;

    // --- Mutation: update counted_qty on each submitted line ---
    for line_input in &req.lines {
        sqlx::query(
            r#"
            UPDATE cycle_count_lines
            SET counted_qty = $1
            WHERE id = $2 AND task_id = $3
            "#,
        )
        .bind(line_input.counted_qty)
        .bind(line_input.line_id)
        .bind(req.task_id)
        .execute(&mut *tx)
        .await?;
    }

    // --- Mutation: move task to 'submitted' ---
    sqlx::query(
        r#"
        UPDATE cycle_count_tasks
        SET status = 'submitted', updated_at = $1
        WHERE id = $2 AND tenant_id = $3
        "#,
    )
    .bind(submitted_at)
    .bind(req.task_id)
    .bind(&req.tenant_id)
    .execute(&mut *tx)
    .await?;

    // --- Build result lines with variance ---
    let result_lines: Vec<SubmittedLine> = build_result_lines(&existing_lines, &req.lines);

    // --- Outbox: emit inventory.cycle_count_submitted ---
    let event_lines: Vec<CycleCountSubmittedLine> = result_lines
        .iter()
        .map(|l| CycleCountSubmittedLine {
            line_id: l.line_id,
            item_id: l.item_id,
            expected_qty: l.expected_qty,
            counted_qty: l.counted_qty,
            variance_qty: l.variance_qty,
        })
        .collect();

    let payload = CycleCountSubmittedPayload {
        task_id: req.task_id,
        tenant_id: req.tenant_id.clone(),
        warehouse_id: task.warehouse_id,
        location_id: task.location_id,
        submitted_at,
        line_count: result_lines.len(),
        lines: event_lines,
    };
    let envelope = build_cycle_count_submitted_envelope(
        event_id,
        req.tenant_id.clone(),
        correlation_id.clone(),
        req.causation_id.clone(),
        payload,
    );
    let envelope_json = serde_json::to_string(&envelope)?;

    sqlx::query(
        r#"
        INSERT INTO inv_outbox
            (event_id, event_type, aggregate_type, aggregate_id, tenant_id,
             payload, correlation_id, causation_id, schema_version)
        VALUES
            ($1, $2, 'cycle_count_task', $3, $4, $5::JSONB, $6, $7, '1.0.0')
        "#,
    )
    .bind(event_id)
    .bind(EVENT_TYPE_CYCLE_COUNT_SUBMITTED)
    .bind(req.task_id.to_string())
    .bind(&req.tenant_id)
    .bind(&envelope_json)
    .bind(&correlation_id)
    .bind(&req.causation_id)
    .execute(&mut *tx)
    .await?;

    // --- Store idempotency key (expires in 7 days) ---
    let result = SubmitResult {
        task_id: req.task_id,
        tenant_id: req.tenant_id.clone(),
        status: "submitted".to_string(),
        submitted_at,
        line_count: result_lines.len(),
        lines: result_lines,
    };
    let response_json = serde_json::to_string(&result)?;
    let expires_at = submitted_at + Duration::days(7);

    sqlx::query(
        r#"
        INSERT INTO inv_idempotency_keys
            (tenant_id, idempotency_key, request_hash, response_body, status_code, expires_at)
        VALUES
            ($1, $2, $3, $4::JSONB, 201, $5)
        "#,
    )
    .bind(&req.tenant_id)
    .bind(&req.idempotency_key)
    .bind(&request_hash)
    .bind(&response_json)
    .bind(expires_at)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok((result, false))
}

// ============================================================================
// Helpers
// ============================================================================

fn validate_request(req: &SubmitRequest) -> Result<(), SubmitError> {
    if req.tenant_id.trim().is_empty() {
        return Err(SubmitError::MissingTenant);
    }
    if req.idempotency_key.trim().is_empty() {
        return Err(SubmitError::MissingIdempotencyKey);
    }
    Ok(())
}

async fn find_idempotency_key(
    pool: &PgPool,
    tenant_id: &str,
    idempotency_key: &str,
) -> Result<Option<IdempotencyRecord>, sqlx::Error> {
    sqlx::query_as::<_, IdempotencyRecord>(
        r#"
        SELECT response_body::TEXT AS response_body, request_hash
        FROM inv_idempotency_keys
        WHERE tenant_id = $1 AND idempotency_key = $2
        "#,
    )
    .bind(tenant_id)
    .bind(idempotency_key)
    .fetch_optional(pool)
    .await
}

/// Merge existing DB lines with caller-supplied counted quantities.
///
/// Lines not in the submit request retain counted_qty = 0 and
/// variance = -expected_qty (all missing = shrinkage).
/// This makes the result deterministic from inputs alone.
fn build_result_lines(existing: &[LineRow], inputs: &[SubmitLineInput]) -> Vec<SubmittedLine> {
    existing
        .iter()
        .map(|line| {
            let counted_qty = inputs
                .iter()
                .find(|i| i.line_id == line.id)
                .map(|i| i.counted_qty)
                .unwrap_or(0);
            let variance_qty = counted_qty - line.expected_qty;
            SubmittedLine {
                line_id: line.id,
                item_id: line.item_id,
                expected_qty: line.expected_qty,
                counted_qty,
                variance_qty,
            }
        })
        .collect()
}

// ============================================================================
// Unit tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_req() -> SubmitRequest {
        SubmitRequest {
            task_id: Uuid::new_v4(),
            tenant_id: "tenant-1".to_string(),
            idempotency_key: "submit-001".to_string(),
            lines: vec![],
            correlation_id: None,
            causation_id: None,
        }
    }

    #[test]
    fn validate_rejects_empty_tenant() {
        let mut r = make_req();
        r.tenant_id = "".to_string();
        assert!(matches!(
            validate_request(&r),
            Err(SubmitError::MissingTenant)
        ));
    }

    #[test]
    fn validate_rejects_empty_idempotency_key() {
        let mut r = make_req();
        r.idempotency_key = " ".to_string();
        assert!(matches!(
            validate_request(&r),
            Err(SubmitError::MissingIdempotencyKey)
        ));
    }

    #[test]
    fn validate_accepts_valid_request() {
        let r = make_req();
        assert!(validate_request(&r).is_ok());
    }

    #[test]
    fn build_result_lines_computes_variance_correctly() {
        let line_id = Uuid::new_v4();
        let item_id = Uuid::new_v4();
        let existing = vec![LineRow {
            id: line_id,
            item_id,
            expected_qty: 50,
        }];
        let inputs = vec![SubmitLineInput {
            line_id,
            counted_qty: 45,
        }];
        let result = build_result_lines(&existing, &inputs);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].counted_qty, 45);
        assert_eq!(result[0].expected_qty, 50);
        assert_eq!(result[0].variance_qty, -5);
    }

    #[test]
    fn build_result_lines_defaults_unsubmitted_to_zero() {
        let line_id = Uuid::new_v4();
        let item_id = Uuid::new_v4();
        let existing = vec![LineRow {
            id: line_id,
            item_id,
            expected_qty: 20,
        }];
        let result = build_result_lines(&existing, &[]);
        assert_eq!(result[0].counted_qty, 0);
        assert_eq!(result[0].variance_qty, -20);
    }
}
