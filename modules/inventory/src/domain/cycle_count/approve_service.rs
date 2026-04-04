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
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

use super::repo;
use crate::events::cycle_count_approved::{
    build_cycle_count_approved_envelope, CycleCountApprovedLine, CycleCountApprovedPayload,
    EVENT_TYPE_CYCLE_COUNT_APPROVED,
};
use crate::events::{build_adjusted_envelope, AdjustedPayload, EVENT_TYPE_ADJUSTED};

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
    if let Some(record) = repo::find_idempotency_key(pool, &req.tenant_id, &req.idempotency_key).await? {
        if record.request_hash != request_hash {
            return Err(ApproveError::ConflictingIdempotencyKey);
        }
        let result: ApproveResult = serde_json::from_str(&record.response_body)?;
        return Ok((result, true));
    }

    // --- Guard: load task, verify tenant, check status ---
    let task = repo::fetch_task(pool, req.task_id, &req.tenant_id)
        .await?
        .ok_or(ApproveError::TaskNotFound)?;

    if task.status != "submitted" {
        return Err(ApproveError::TaskNotSubmitted {
            current_status: task.status,
        });
    }

    // --- Load all lines with item SKU (for adjustment payloads) ---
    let lines = repo::fetch_lines_with_sku(pool, req.task_id).await?;

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

        // Step 1: Insert ledger row
        let ledger_entry_id = repo::insert_ledger_row(
            &mut tx,
            &req.tenant_id,
            line.item_id,
            task.warehouse_id,
            task.location_id,
            variance_qty,
            adj_event_id,
            EVENT_TYPE_ADJUSTED,
            approved_at,
        ).await?;

        // Step 2: Insert inv_adjustments business-key row
        let adjustment_id = repo::insert_adjustment(
            &mut tx,
            &req.tenant_id,
            line.item_id,
            task.warehouse_id,
            task.location_id,
            variance_qty,
            adj_event_id,
            ledger_entry_id,
            approved_at,
        ).await?;

        // Step 3: Upsert item_on_hand (location-specific)
        repo::upsert_on_hand(
            &mut tx,
            &req.tenant_id,
            line.item_id,
            task.warehouse_id,
            task.location_id,
            variance_qty,
            ledger_entry_id,
        ).await?;

        // Step 4: Upsert item_on_hand_by_status (available bucket)
        repo::upsert_available_bucket(
            &mut tx,
            &req.tenant_id,
            line.item_id,
            task.warehouse_id,
            variance_qty,
        ).await?;

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

        repo::insert_outbox_event(
            &mut tx,
            adj_event_id,
            EVENT_TYPE_ADJUSTED,
            "inventory_item",
            &line.item_id.to_string(),
            &req.tenant_id,
            &adj_envelope_json,
            &correlation_id,
            req.causation_id.as_deref(),
        ).await?;

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
    repo::update_task_approved(&mut tx, approved_at, req.task_id, &req.tenant_id).await?;

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

    repo::insert_outbox_event(
        &mut tx,
        approve_event_id,
        EVENT_TYPE_CYCLE_COUNT_APPROVED,
        "cycle_count_task",
        &req.task_id.to_string(),
        &req.tenant_id,
        &approve_envelope_json,
        &correlation_id,
        req.causation_id.as_deref(),
    ).await?;

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

    repo::store_idempotency_key(
        &mut tx,
        &req.tenant_id,
        &req.idempotency_key,
        &request_hash,
        &response_json,
        expires_at,
    ).await?;

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
        assert!(matches!(
            validate_request(&r),
            Err(ApproveError::MissingTenant)
        ));
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
