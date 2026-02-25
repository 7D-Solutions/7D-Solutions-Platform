//! Cycle count approve service.
//!
//! Approval is the boundary where physical count variances become inventory
//! movements. For each line with a non-zero variance (counted_qty − expected_qty),
//! this service:
//!   1. Inserts a ledger row (entry_type = 'adjusted', append-only)
//!   2. Inserts an inv_adjustments business-key row
//!   3. Upserts item_on_hand + item_on_hand_by_status projections
//!   4. Emits inventory.adjusted to inv_outbox (one event per adjusted line)
//!
//! Then moves the task from 'submitted' → 'approved' and emits
//! inventory.cycle_count_approved.
//!
//! All mutations occur in one transaction — either everything commits or nothing.
//!
//! Pattern: Guard → Mutation → Outbox (all in one transaction)
//!
//! Guards:
//!   - tenant_id and idempotency_key must be non-empty
//!   - task must exist and belong to this tenant
//!   - task must be in 'submitted' status

use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};
use thiserror::Error;
use uuid::Uuid;

use crate::events::{
    AdjustedPayload, EVENT_TYPE_ADJUSTED, build_adjusted_envelope,
};
use crate::events::cycle_count_approved::{
    build_cycle_count_approved_envelope, CycleCountApprovedLine, CycleCountApprovedPayload,
    EVENT_TYPE_CYCLE_COUNT_APPROVED,
};

// ============================================================================
// Types
// ============================================================================

/// Request to approve a submitted cycle count task.
#[derive(Debug, Serialize, Deserialize)]
pub struct ApproveRequest {
    pub task_id: Uuid,
    pub tenant_id: String,
    pub idempotency_key: String,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
}

/// One line in the approve result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovedLine {
    pub line_id: Uuid,
    pub item_id: Uuid,
    pub expected_qty: i64,
    pub counted_qty: i64,
    /// counted_qty − expected_qty
    pub variance_qty: i64,
    /// None when variance_qty == 0 (no adjustment created)
    pub adjustment_id: Option<Uuid>,
}

/// Returned on successful or replayed approve.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApproveResult {
    pub task_id: Uuid,
    pub tenant_id: String,
    pub status: String,
    pub approved_at: chrono::DateTime<Utc>,
    pub line_count: usize,
    pub adjustment_count: usize,
    pub lines: Vec<ApprovedLine>,
}

#[derive(Debug, Error)]
pub enum ApproveError {
    #[error("tenant_id is required")]
    MissingTenant,

    #[error("idempotency_key is required")]
    MissingIdempotencyKey,

    #[error("task not found or does not belong to this tenant")]
    TaskNotFound,

    #[error("task is not submitted (current status: {current_status})")]
    TaskNotSubmitted { current_status: String },

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
struct LineWithSku {
    id: Uuid,
    item_id: Uuid,
    expected_qty: i64,
    counted_qty: Option<i64>,
    sku: String,
}

#[derive(sqlx::FromRow)]
struct IdempotencyRecord {
    response_body: String,
    request_hash: String,
}

#[derive(sqlx::FromRow)]
struct LedgerInserted {
    id: i64,
}

// ============================================================================
// Service
// ============================================================================

/// Approve a submitted cycle count task.
///
/// Returns `(ApproveResult, is_replay)`.
/// - `is_replay = false`: task moved to approved, adjustments created; HTTP 201.
/// - `is_replay = true`:  idempotency key matched; HTTP 200 with stored result.
pub async fn approve_cycle_count(
    pool: &PgPool,
    req: &ApproveRequest,
) -> Result<(ApproveResult, bool), ApproveError> {
    // --- Stateless validation ---
    validate_request(req)?;

    // --- Idempotency check (fast path for replays) ---
    let request_hash = serde_json::to_string(req)?;
    if let Some(record) = find_idempotency_key(pool, &req.tenant_id, &req.idempotency_key).await? {
        if record.request_hash != request_hash {
            return Err(ApproveError::ConflictingIdempotencyKey);
        }
        let result: ApproveResult = serde_json::from_str(&record.response_body)?;
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

    let task = task.ok_or(ApproveError::TaskNotFound)?;

    if task.status != "submitted" {
        return Err(ApproveError::TaskNotSubmitted {
            current_status: task.status,
        });
    }

    // --- Load all lines with item SKU (for adjustment payloads) ---
    let lines: Vec<LineWithSku> = sqlx::query_as::<_, LineWithSku>(
        r#"
        SELECT ccl.id, ccl.item_id, ccl.expected_qty, ccl.counted_qty, i.sku
        FROM cycle_count_lines ccl
        JOIN items i ON i.id = ccl.item_id
        WHERE ccl.task_id = $1
        "#,
    )
    .bind(req.task_id)
    .fetch_all(pool)
    .await?;

    let approved_at = Utc::now();
    let approve_event_id = Uuid::new_v4();
    let correlation_id = req
        .correlation_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let mut tx = pool.begin().await?;

    let mut approved_lines: Vec<ApprovedLine> = Vec::with_capacity(lines.len());
    let mut adjustment_count: usize = 0;

    // --- Mutation: create adjustment entries for non-zero variances ---
    for line in &lines {
        let counted_qty = line.counted_qty.unwrap_or(line.expected_qty);
        let variance_qty = counted_qty - line.expected_qty;

        if variance_qty == 0 {
            approved_lines.push(ApprovedLine {
                line_id: line.id,
                item_id: line.item_id,
                expected_qty: line.expected_qty,
                counted_qty,
                variance_qty,
                adjustment_id: None,
            });
            continue;
        }

        let adj_event_id = Uuid::new_v4();

        // Step 1: Insert ledger row (entry_type = 'adjusted', zero cost in v1)
        let ledger = sqlx::query_as::<_, LedgerInserted>(
            r#"
            INSERT INTO inventory_ledger
                (tenant_id, item_id, warehouse_id, location_id, entry_type,
                 quantity, unit_cost_minor, currency,
                 source_event_id, source_event_type,
                 reference_type, notes, posted_at)
            VALUES
                ($1, $2, $3, $4, 'adjusted', $5, 0, 'usd', $6, $7,
                 'adjustment', 'cycle_count', $8)
            RETURNING id
            "#,
        )
        .bind(&req.tenant_id)
        .bind(line.item_id)
        .bind(task.warehouse_id)
        .bind(task.location_id)
        .bind(variance_qty)
        .bind(adj_event_id)
        .bind(EVENT_TYPE_ADJUSTED)
        .bind(approved_at)
        .fetch_one(&mut *tx)
        .await?;

        let ledger_entry_id = ledger.id;

        // Step 2: Insert inv_adjustments row (business key)
        let adjustment_id = sqlx::query_scalar::<_, Uuid>(
            r#"
            INSERT INTO inv_adjustments
                (tenant_id, item_id, warehouse_id, location_id,
                 quantity_delta, reason, event_id, ledger_entry_id, adjusted_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            RETURNING id
            "#,
        )
        .bind(&req.tenant_id)
        .bind(line.item_id)
        .bind(task.warehouse_id)
        .bind(task.location_id)
        .bind(variance_qty)
        .bind("cycle_count")
        .bind(adj_event_id)
        .bind(ledger_entry_id)
        .bind(approved_at)
        .fetch_one(&mut *tx)
        .await?;

        // Step 3: Upsert item_on_hand (location-specific, since cycle count tasks
        //         always have a location)
        upsert_on_hand(
            &mut tx,
            &req.tenant_id,
            line.item_id,
            task.warehouse_id,
            task.location_id,
            variance_qty,
            ledger_entry_id,
        )
        .await?;

        // Step 4: Upsert item_on_hand_by_status (available bucket)
        upsert_available_bucket(
            &mut tx,
            &req.tenant_id,
            line.item_id,
            task.warehouse_id,
            variance_qty,
        )
        .await?;

        // Step 5: Outbox — inventory.adjusted (one per adjusted line)
        let adj_payload = AdjustedPayload {
            adjustment_id,
            tenant_id: req.tenant_id.clone(),
            item_id: line.item_id,
            sku: line.sku.clone(),
            warehouse_id: task.warehouse_id,
            quantity_delta: variance_qty,
            reason: "cycle_count".to_string(),
            adjusted_at: approved_at,
        };
        let adj_envelope = build_adjusted_envelope(
            adj_event_id,
            req.tenant_id.clone(),
            correlation_id.clone(),
            req.causation_id.clone(),
            adj_payload,
        );
        let adj_envelope_json = serde_json::to_string(&adj_envelope)?;

        sqlx::query(
            r#"
            INSERT INTO inv_outbox
                (event_id, event_type, aggregate_type, aggregate_id, tenant_id,
                 payload, correlation_id, causation_id, schema_version)
            VALUES
                ($1, $2, 'inventory_item', $3, $4, $5::JSONB, $6, $7, '1.0.0')
            "#,
        )
        .bind(adj_event_id)
        .bind(EVENT_TYPE_ADJUSTED)
        .bind(line.item_id.to_string())
        .bind(&req.tenant_id)
        .bind(&adj_envelope_json)
        .bind(&correlation_id)
        .bind(&req.causation_id)
        .execute(&mut *tx)
        .await?;

        adjustment_count += 1;
        approved_lines.push(ApprovedLine {
            line_id: line.id,
            item_id: line.item_id,
            expected_qty: line.expected_qty,
            counted_qty,
            variance_qty,
            adjustment_id: Some(adjustment_id),
        });
    }

    // --- Mutation: move task to 'approved' ---
    sqlx::query(
        r#"
        UPDATE cycle_count_tasks
        SET status = 'approved', updated_at = $1
        WHERE id = $2 AND tenant_id = $3
        "#,
    )
    .bind(approved_at)
    .bind(req.task_id)
    .bind(&req.tenant_id)
    .execute(&mut *tx)
    .await?;

    // --- Outbox: inventory.cycle_count_approved ---
    let approve_payload = CycleCountApprovedPayload {
        task_id: req.task_id,
        tenant_id: req.tenant_id.clone(),
        warehouse_id: task.warehouse_id,
        location_id: task.location_id,
        approved_at,
        line_count: approved_lines.len(),
        adjustment_count,
        lines: approved_lines
            .iter()
            .map(|l| CycleCountApprovedLine {
                line_id: l.line_id,
                item_id: l.item_id,
                expected_qty: l.expected_qty,
                counted_qty: l.counted_qty,
                variance_qty: l.variance_qty,
                adjustment_id: l.adjustment_id,
            })
            .collect(),
    };
    let approve_envelope = build_cycle_count_approved_envelope(
        approve_event_id,
        req.tenant_id.clone(),
        correlation_id.clone(),
        req.causation_id.clone(),
        approve_payload,
    );
    let approve_envelope_json = serde_json::to_string(&approve_envelope)?;

    sqlx::query(
        r#"
        INSERT INTO inv_outbox
            (event_id, event_type, aggregate_type, aggregate_id, tenant_id,
             payload, correlation_id, causation_id, schema_version)
        VALUES
            ($1, $2, 'cycle_count_task', $3, $4, $5::JSONB, $6, $7, '1.0.0')
        "#,
    )
    .bind(approve_event_id)
    .bind(EVENT_TYPE_CYCLE_COUNT_APPROVED)
    .bind(req.task_id.to_string())
    .bind(&req.tenant_id)
    .bind(&approve_envelope_json)
    .bind(&correlation_id)
    .bind(&req.causation_id)
    .execute(&mut *tx)
    .await?;

    // --- Store idempotency key (expires in 7 days) ---
    let result = ApproveResult {
        task_id: req.task_id,
        tenant_id: req.tenant_id.clone(),
        status: "approved".to_string(),
        approved_at,
        line_count: approved_lines.len(),
        adjustment_count,
        lines: approved_lines,
    };
    let response_json = serde_json::to_string(&result)?;
    let expires_at = approved_at + Duration::days(7);

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

fn validate_request(req: &ApproveRequest) -> Result<(), ApproveError> {
    if req.tenant_id.trim().is_empty() {
        return Err(ApproveError::MissingTenant);
    }
    if req.idempotency_key.trim().is_empty() {
        return Err(ApproveError::MissingIdempotencyKey);
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

/// Upsert item_on_hand projection for a location-specific row.
async fn upsert_on_hand(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
    location_id: Uuid,
    delta: i64,
    ledger_entry_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO item_on_hand
            (tenant_id, item_id, warehouse_id, location_id,
             quantity_on_hand, available_status_on_hand,
             total_cost_minor, currency, last_ledger_entry_id, projected_at)
        VALUES ($1, $2, $3, $4, $5, $5, 0, 'usd', $6, NOW())
        ON CONFLICT (tenant_id, item_id, warehouse_id, location_id)
        WHERE location_id IS NOT NULL
        DO UPDATE
            SET quantity_on_hand         = item_on_hand.quantity_on_hand + $5,
                available_status_on_hand = item_on_hand.available_status_on_hand + $5,
                last_ledger_entry_id     = $6,
                projected_at             = NOW()
        "#,
    )
    .bind(tenant_id)
    .bind(item_id)
    .bind(warehouse_id)
    .bind(location_id)
    .bind(delta)
    .bind(ledger_entry_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Update item_on_hand_by_status (available bucket) after an adjustment.
///
/// Positive delta: upsert (create or increment).
/// Negative delta: UPDATE only to avoid CHECK constraint violation on INSERT.
async fn upsert_available_bucket(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
    delta: i64,
) -> Result<(), sqlx::Error> {
    if delta >= 0 {
        sqlx::query(
            r#"
            INSERT INTO item_on_hand_by_status
                (tenant_id, item_id, warehouse_id, status, quantity_on_hand)
            VALUES ($1, $2, $3, 'available', $4)
            ON CONFLICT (tenant_id, item_id, warehouse_id, status) DO UPDATE
                SET quantity_on_hand = item_on_hand_by_status.quantity_on_hand + $4,
                    updated_at       = NOW()
            "#,
        )
        .bind(tenant_id)
        .bind(item_id)
        .bind(warehouse_id)
        .bind(delta)
        .execute(&mut **tx)
        .await?;
    } else {
        sqlx::query(
            r#"
            UPDATE item_on_hand_by_status
            SET quantity_on_hand = GREATEST(0, quantity_on_hand + $4),
                updated_at       = NOW()
            WHERE tenant_id    = $1
              AND item_id      = $2
              AND warehouse_id = $3
              AND status       = 'available'
            "#,
        )
        .bind(tenant_id)
        .bind(item_id)
        .bind(warehouse_id)
        .bind(delta)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

// ============================================================================
// Unit tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_req() -> ApproveRequest {
        ApproveRequest {
            task_id: Uuid::new_v4(),
            tenant_id: "tenant-1".to_string(),
            idempotency_key: "approve-001".to_string(),
            correlation_id: None,
            causation_id: None,
        }
    }

    #[test]
    fn validate_rejects_empty_tenant() {
        let mut r = make_req();
        r.tenant_id = "".to_string();
        assert!(matches!(validate_request(&r), Err(ApproveError::MissingTenant)));
    }

    #[test]
    fn validate_rejects_empty_idempotency_key() {
        let mut r = make_req();
        r.idempotency_key = "  ".to_string();
        assert!(matches!(
            validate_request(&r),
            Err(ApproveError::MissingIdempotencyKey)
        ));
    }

    #[test]
    fn validate_accepts_valid_request() {
        let r = make_req();
        assert!(validate_request(&r).is_ok());
    }
}
