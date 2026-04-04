//! Atomic inter-warehouse stock transfer service.
//!
//! Invariants:
//! - Both legs (transfer_out + transfer_in) are written in a single transaction.
//! - FIFO consumption on the source side; new cost layer created at destination.
//! - No partial transfers: the transaction either fully commits or fully rolls back.
//! - Idempotency key prevents double-processing on retry.
//!
//! Pattern: Guard → Lock → FIFO → Mutation → Outbox (all in one transaction).

use chrono::{Duration, Utc};
use event_bus::TracingContext;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    domain::{
        fifo::{self, AvailableLayer, FifoError},
        guards::{guard_item_active, guard_quantity_positive, GuardError},
        projections::on_hand,
        transfer_repo,
    },
    events::{
        contracts::{build_transfer_completed_envelope, ConsumedLayer, TransferCompletedPayload},
        EVENT_TYPE_TRANSFER_COMPLETED,
    },
};

// ============================================================================
// Types
// ============================================================================

/// Input for POST /api/inventory/transfers
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TransferRequest {
    pub tenant_id: String,
    pub item_id: Uuid,
    pub from_warehouse_id: Uuid,
    pub to_warehouse_id: Uuid,
    /// Quantity to transfer (must be > 0; in base_uom units)
    pub quantity: i64,
    pub currency: String,
    /// Caller-supplied idempotency key (required; scoped per tenant)
    pub idempotency_key: String,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
}

/// Result returned on successful or replayed transfer
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TransferResult {
    /// Stable business key for this transfer (= inv_transfers.id)
    pub transfer_id: Uuid,
    /// Outbox event id
    pub event_id: Uuid,
    /// Ledger entry id for the 'transfer_out' leg (source debit)
    pub issue_ledger_id: i64,
    /// Ledger entry id for the 'transfer_in' leg (destination credit)
    pub receipt_ledger_id: i64,
    pub tenant_id: String,
    pub item_id: Uuid,
    pub from_warehouse_id: Uuid,
    pub to_warehouse_id: Uuid,
    pub quantity: i64,
    /// Total FIFO cost consumed from source layers
    pub total_cost_minor: i64,
    pub currency: String,
    /// FIFO breakdown consumed from source
    pub consumed_layers: Vec<ConsumedLayer>,
    pub transferred_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Error)]
pub enum TransferError {
    #[error("Guard failed: {0}")]
    Guard(#[from] GuardError),

    #[error("Source and destination warehouse must be different")]
    SameWarehouse,

    #[error("Insufficient stock: requested {requested}, available {available}")]
    InsufficientQuantity { requested: i64, available: i64 },

    #[error("FIFO engine error: {0}")]
    Fifo(#[from] FifoError),

    #[error("Idempotency key conflict: same key used with a different request body")]
    ConflictingIdempotencyKey,

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ============================================================================
// Service
// ============================================================================

/// Process an inter-warehouse stock transfer atomically.
///
/// Returns `(TransferResult, is_replay)`.
/// - `is_replay = false`: new transfer created; HTTP 201.
/// - `is_replay = true`:  idempotency key matched; HTTP 200.
pub async fn process_transfer(
    pool: &PgPool,
    req: &TransferRequest,
    tracing_ctx: Option<&TracingContext>,
) -> Result<(TransferResult, bool), TransferError> {
    // --- Stateless input validation ---
    validate_request(req)?;

    let request_hash = serde_json::to_string(req)?;

    // --- Idempotency fast-path ---
    if let Some(record) = transfer_repo::find_idempotency_key(pool, &req.tenant_id, &req.idempotency_key).await? {
        if record.request_hash != request_hash {
            return Err(TransferError::ConflictingIdempotencyKey);
        }
        let result: TransferResult = serde_json::from_str(&record.response_body)?;
        return Ok((result, true));
    }

    // --- Guard: item must exist and be active ---
    let item = guard_item_active(pool, req.item_id, &req.tenant_id).await?;

    let transferred_at = Utc::now();
    let event_id = Uuid::new_v4();
    let correlation_id = req
        .correlation_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let mut tx = pool.begin().await?;

    // --- Lock FIFO layers on source warehouse (deterministic FIFO order) ---
    let layer_rows = transfer_repo::lock_fifo_layers(&mut tx, &req.tenant_id, req.item_id, req.from_warehouse_id).await?;

    let available_layers: Vec<AvailableLayer> = layer_rows
        .iter()
        .map(|r| AvailableLayer {
            layer_id: r.id,
            quantity_remaining: r.quantity_remaining,
            unit_cost_minor: r.unit_cost_minor,
        })
        .collect();

    let sum_remaining: i64 = available_layers.iter().map(|l| l.quantity_remaining).sum();

    // Available = total remaining − reserved
    let quantity_reserved = transfer_repo::fetch_quantity_reserved(&mut tx, &req.tenant_id, req.item_id, req.from_warehouse_id).await?;

    let net_available = sum_remaining - quantity_reserved;
    if net_available < req.quantity {
        return Err(TransferError::InsufficientQuantity {
            requested: req.quantity,
            available: net_available,
        });
    }

    // --- FIFO consumption from source ---
    let consumed = fifo::consume_fifo(&available_layers, req.quantity)?;
    let total_cost_minor: i64 = consumed.iter().map(|c| c.extended_cost_minor).sum();

    // Pre/post cost totals for source on-hand absolute set
    let pre_cost: i64 = available_layers
        .iter()
        .map(|l| l.quantity_remaining * l.unit_cost_minor)
        .sum();
    let post_cost = (pre_cost - total_cost_minor).max(0);
    let new_source_on_hand = sum_remaining - req.quantity;

    // Unique source event id for the 'transfer_out' ledger row
    let out_event_id = Uuid::new_v4();

    // --- Step 1: Insert 'transfer_out' ledger row (source, negative qty) ---
    let transfer_id = Uuid::new_v4();

    let issue_ledger_id = transfer_repo::insert_transfer_out_ledger(
        &mut tx,
        &req.tenant_id,
        req.item_id,
        req.from_warehouse_id,
        -req.quantity,
        &req.currency,
        out_event_id,
        EVENT_TYPE_TRANSFER_COMPLETED,
        &transfer_id.to_string(),
        transferred_at,
    ).await?;

    // --- Step 2: FIFO layer consumptions + decrement source layer quantities ---
    for c in &consumed {
        transfer_repo::insert_layer_consumption(
            &mut tx,
            c.layer_id,
            issue_ledger_id,
            c.quantity,
            c.unit_cost_minor,
            transferred_at,
        ).await?;

        transfer_repo::decrement_layer(&mut tx, c.quantity, transferred_at, c.layer_id).await?;
    }

    // --- Step 3: Update source on-hand projection (absolute set from FIFO sums) ---
    on_hand::upsert_after_issue(
        &mut tx,
        &req.tenant_id,
        req.item_id,
        req.from_warehouse_id,
        new_source_on_hand,
        post_cost,
        &req.currency,
        issue_ledger_id,
    )
    .await
    .map_err(TransferError::Database)?;

    on_hand::set_available_bucket(
        &mut tx,
        &req.tenant_id,
        req.item_id,
        req.from_warehouse_id,
        new_source_on_hand,
    )
    .await
    .map_err(TransferError::Database)?;

    // Unique destination event id for the 'transfer_in' ledger row
    let in_event_id = Uuid::new_v4();

    // Weighted average unit cost for the destination FIFO layer
    let avg_unit_cost = if req.quantity > 0 {
        total_cost_minor / req.quantity
    } else {
        0
    };

    // --- Step 4: Insert 'transfer_in' ledger row (destination, positive qty) ---
    let receipt_ledger_id = transfer_repo::insert_transfer_in_ledger(
        &mut tx,
        &req.tenant_id,
        req.item_id,
        req.to_warehouse_id,
        req.quantity,
        avg_unit_cost,
        &req.currency,
        in_event_id,
        EVENT_TYPE_TRANSFER_COMPLETED,
        &transfer_id.to_string(),
        transferred_at,
    ).await?;

    // --- Step 5: Create new FIFO layer at destination ---
    transfer_repo::insert_destination_fifo_layer(
        &mut tx,
        &req.tenant_id,
        req.item_id,
        req.to_warehouse_id,
        receipt_ledger_id,
        transferred_at,
        req.quantity,
        avg_unit_cost,
        &req.currency,
    ).await?;

    // --- Step 6: Update destination on-hand projection ---
    on_hand::upsert_after_receipt(
        &mut tx,
        &req.tenant_id,
        req.item_id,
        req.to_warehouse_id,
        None, // warehouse-level (no location)
        req.quantity,
        avg_unit_cost,
        &req.currency,
        receipt_ledger_id,
    )
    .await
    .map_err(TransferError::Database)?;

    on_hand::add_to_available_bucket(
        &mut tx,
        &req.tenant_id,
        req.item_id,
        req.to_warehouse_id,
        req.quantity,
    )
    .await
    .map_err(TransferError::Database)?;

    // --- Step 7: Insert inv_transfers business record ---
    transfer_repo::insert_transfer_record(
        &mut tx,
        transfer_id,
        &req.tenant_id,
        req.item_id,
        req.from_warehouse_id,
        req.to_warehouse_id,
        req.quantity,
        event_id,
        issue_ledger_id,
        receipt_ledger_id,
        transferred_at,
    ).await?;

    // --- Step 8: Enqueue outbox event ---
    let payload = TransferCompletedPayload {
        transfer_id,
        tenant_id: req.tenant_id.clone(),
        item_id: req.item_id,
        sku: item.sku,
        from_warehouse_id: req.from_warehouse_id,
        to_warehouse_id: req.to_warehouse_id,
        quantity: req.quantity,
        transferred_at,
    };

    let default_ctx = TracingContext::default();
    let ctx = tracing_ctx.unwrap_or(&default_ctx);
    let envelope = build_transfer_completed_envelope(
        event_id,
        req.tenant_id.clone(),
        correlation_id.clone(),
        req.causation_id.clone(),
        payload,
    )
    .with_tracing_context(ctx);
    let envelope_json = serde_json::to_string(&envelope)?;

    transfer_repo::insert_outbox_event(
        &mut tx,
        event_id,
        EVENT_TYPE_TRANSFER_COMPLETED,
        &req.item_id.to_string(),
        &req.tenant_id,
        &envelope_json,
        &correlation_id,
        req.causation_id.as_deref(),
    ).await?;

    // --- Step 9: Build result and store idempotency key ---
    let result = TransferResult {
        transfer_id,
        event_id,
        issue_ledger_id,
        receipt_ledger_id,
        tenant_id: req.tenant_id.clone(),
        item_id: req.item_id,
        from_warehouse_id: req.from_warehouse_id,
        to_warehouse_id: req.to_warehouse_id,
        quantity: req.quantity,
        total_cost_minor,
        currency: req.currency.clone(),
        consumed_layers: consumed,
        transferred_at,
    };

    let response_json = serde_json::to_string(&result)?;
    let expires_at = transferred_at + Duration::days(7);

    transfer_repo::store_idempotency_key(
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

fn validate_request(req: &TransferRequest) -> Result<(), TransferError> {
    if req.tenant_id.trim().is_empty() {
        return Err(TransferError::Guard(GuardError::Validation(
            "tenant_id is required".to_string(),
        )));
    }
    if req.idempotency_key.trim().is_empty() {
        return Err(TransferError::Guard(GuardError::Validation(
            "idempotency_key is required".to_string(),
        )));
    }
    if req.currency.trim().is_empty() {
        return Err(TransferError::Guard(GuardError::Validation(
            "currency is required".to_string(),
        )));
    }
    if req.from_warehouse_id == req.to_warehouse_id {
        return Err(TransferError::SameWarehouse);
    }
    guard_quantity_positive(req.quantity).map_err(TransferError::Guard)?;
    Ok(())
}

// ============================================================================
// Unit tests (stateless validation only)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_req() -> TransferRequest {
        TransferRequest {
            tenant_id: "tenant-1".to_string(),
            item_id: Uuid::new_v4(),
            from_warehouse_id: Uuid::new_v4(),
            to_warehouse_id: Uuid::new_v4(),
            quantity: 10,
            currency: "usd".to_string(),
            idempotency_key: "xfer-001".to_string(),
            correlation_id: None,
            causation_id: None,
        }
    }

    #[test]
    fn rejects_empty_tenant() {
        let mut r = valid_req();
        r.tenant_id = "".to_string();
        assert!(matches!(validate_request(&r), Err(TransferError::Guard(_))));
    }

    #[test]
    fn rejects_empty_idempotency_key() {
        let mut r = valid_req();
        r.idempotency_key = "  ".to_string();
        assert!(matches!(validate_request(&r), Err(TransferError::Guard(_))));
    }

    #[test]
    fn rejects_zero_quantity() {
        let mut r = valid_req();
        r.quantity = 0;
        assert!(matches!(validate_request(&r), Err(TransferError::Guard(_))));
    }

    #[test]
    fn rejects_negative_quantity() {
        let mut r = valid_req();
        r.quantity = -5;
        assert!(matches!(validate_request(&r), Err(TransferError::Guard(_))));
    }

    #[test]
    fn rejects_same_warehouse() {
        let mut r = valid_req();
        r.to_warehouse_id = r.from_warehouse_id;
        assert!(matches!(
            validate_request(&r),
            Err(TransferError::SameWarehouse)
        ));
    }

    #[test]
    fn rejects_empty_currency() {
        let mut r = valid_req();
        r.currency = "".to_string();
        assert!(matches!(validate_request(&r), Err(TransferError::Guard(_))));
    }

    #[test]
    fn accepts_valid_request() {
        assert!(validate_request(&valid_req()).is_ok());
    }
}
