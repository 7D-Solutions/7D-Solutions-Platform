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
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

use crate::{
    domain::{
        fifo::{self, AvailableLayer, FifoError},
        guards::{GuardError, guard_item_active, guard_quantity_positive},
        projections::on_hand,
    },
    events::{
        contracts::{ConsumedLayer, TransferCompletedPayload, build_transfer_completed_envelope},
        EVENT_TYPE_TRANSFER_COMPLETED,
    },
};

// ============================================================================
// Types
// ============================================================================

/// Input for POST /api/inventory/transfers
#[derive(Debug, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
// Internal DB row types
// ============================================================================

#[derive(sqlx::FromRow)]
struct LedgerRow {
    id: i64,
}

#[derive(sqlx::FromRow)]
struct IdempotencyRecord {
    response_body: String,
    request_hash: String,
}

#[derive(sqlx::FromRow)]
struct LayerRow {
    id: Uuid,
    quantity_remaining: i64,
    unit_cost_minor: i64,
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
) -> Result<(TransferResult, bool), TransferError> {
    // --- Stateless input validation ---
    validate_request(req)?;

    let request_hash = serde_json::to_string(req)?;

    // --- Idempotency fast-path ---
    if let Some(record) = find_idempotency_key(pool, &req.tenant_id, &req.idempotency_key).await? {
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
    let layer_rows = sqlx::query_as::<_, LayerRow>(
        r#"
        SELECT id, quantity_remaining, unit_cost_minor
        FROM inventory_layers
        WHERE tenant_id     = $1
          AND item_id       = $2
          AND warehouse_id  = $3
          AND quantity_remaining > 0
        ORDER BY received_at ASC, ledger_entry_id ASC
        FOR UPDATE
        "#,
    )
    .bind(&req.tenant_id)
    .bind(req.item_id)
    .bind(req.from_warehouse_id)
    .fetch_all(&mut *tx)
    .await?;

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
    let quantity_reserved: i64 = sqlx::query_scalar(
        r#"
        SELECT COALESCE(quantity_reserved, 0)
        FROM item_on_hand
        WHERE tenant_id    = $1
          AND item_id      = $2
          AND warehouse_id = $3
          AND location_id IS NULL
        "#,
    )
    .bind(&req.tenant_id)
    .bind(req.item_id)
    .bind(req.from_warehouse_id)
    .fetch_optional(&mut *tx)
    .await?
    .unwrap_or(0i64);

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

    let out_ledger = sqlx::query_as::<_, LedgerRow>(
        r#"
        INSERT INTO inventory_ledger
            (tenant_id, item_id, warehouse_id, location_id, entry_type, quantity,
             unit_cost_minor, currency, source_event_id, source_event_type,
             reference_type, reference_id, posted_at)
        VALUES
            ($1, $2, $3, NULL, 'transfer_out', $4, 0, $5, $6, $7, 'transfer', $8, $9)
        RETURNING id
        "#,
    )
    .bind(&req.tenant_id)
    .bind(req.item_id)
    .bind(req.from_warehouse_id)
    .bind(-req.quantity) // negative = stock out
    .bind(&req.currency)
    .bind(out_event_id)
    .bind(EVENT_TYPE_TRANSFER_COMPLETED)
    .bind(transfer_id.to_string())
    .bind(transferred_at)
    .fetch_one(&mut *tx)
    .await?;

    let issue_ledger_id = out_ledger.id;

    // --- Step 2: FIFO layer consumptions + decrement source layer quantities ---
    for c in &consumed {
        sqlx::query(
            r#"
            INSERT INTO layer_consumptions
                (layer_id, ledger_entry_id, quantity_consumed, unit_cost_minor, consumed_at)
            VALUES ($1, $2, $3, $4, $5)
            "#,
        )
        .bind(c.layer_id)
        .bind(issue_ledger_id)
        .bind(c.quantity)
        .bind(c.unit_cost_minor)
        .bind(transferred_at)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            r#"
            UPDATE inventory_layers
            SET quantity_remaining = quantity_remaining - $1,
                exhausted_at = CASE
                    WHEN quantity_remaining - $1 = 0 THEN $2
                    ELSE exhausted_at
                END
            WHERE id = $3
            "#,
        )
        .bind(c.quantity)
        .bind(transferred_at)
        .bind(c.layer_id)
        .execute(&mut *tx)
        .await?;
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
    let in_ledger = sqlx::query_as::<_, LedgerRow>(
        r#"
        INSERT INTO inventory_ledger
            (tenant_id, item_id, warehouse_id, location_id, entry_type, quantity,
             unit_cost_minor, currency, source_event_id, source_event_type,
             reference_type, reference_id, posted_at)
        VALUES
            ($1, $2, $3, NULL, 'transfer_in', $4, $5, $6, $7, $8, 'transfer', $9, $10)
        RETURNING id
        "#,
    )
    .bind(&req.tenant_id)
    .bind(req.item_id)
    .bind(req.to_warehouse_id)
    .bind(req.quantity) // positive = stock in
    .bind(avg_unit_cost)
    .bind(&req.currency)
    .bind(in_event_id)
    .bind(EVENT_TYPE_TRANSFER_COMPLETED)
    .bind(transfer_id.to_string())
    .bind(transferred_at)
    .fetch_one(&mut *tx)
    .await?;

    let receipt_ledger_id = in_ledger.id;

    // --- Step 5: Create new FIFO layer at destination ---
    sqlx::query(
        r#"
        INSERT INTO inventory_layers
            (tenant_id, item_id, warehouse_id, ledger_entry_id, received_at,
             quantity_received, quantity_remaining, unit_cost_minor, currency)
        VALUES
            ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        "#,
    )
    .bind(&req.tenant_id)
    .bind(req.item_id)
    .bind(req.to_warehouse_id)
    .bind(receipt_ledger_id)
    .bind(transferred_at)
    .bind(req.quantity)
    .bind(req.quantity)
    .bind(avg_unit_cost)
    .bind(&req.currency)
    .execute(&mut *tx)
    .await?;

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
    sqlx::query(
        r#"
        INSERT INTO inv_transfers
            (id, tenant_id, item_id, from_warehouse_id, to_warehouse_id,
             quantity, event_id, issue_ledger_id, receipt_ledger_id, transferred_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        "#,
    )
    .bind(transfer_id)
    .bind(&req.tenant_id)
    .bind(req.item_id)
    .bind(req.from_warehouse_id)
    .bind(req.to_warehouse_id)
    .bind(req.quantity)
    .bind(event_id)
    .bind(issue_ledger_id)
    .bind(receipt_ledger_id)
    .bind(transferred_at)
    .execute(&mut *tx)
    .await?;

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

    let envelope = build_transfer_completed_envelope(
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
            ($1, $2, 'inventory_item', $3, $4, $5::JSONB, $6, $7, '1.0.0')
        "#,
    )
    .bind(event_id)
    .bind(EVENT_TYPE_TRANSFER_COMPLETED)
    .bind(req.item_id.to_string())
    .bind(&req.tenant_id)
    .bind(&envelope_json)
    .bind(&correlation_id)
    .bind(&req.causation_id)
    .execute(&mut *tx)
    .await?;

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

    sqlx::query(
        r#"
        INSERT INTO inv_idempotency_keys
            (tenant_id, idempotency_key, request_hash, response_body, status_code, expires_at)
        VALUES ($1, $2, $3, $4::JSONB, 201, $5)
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
        assert!(matches!(validate_request(&r), Err(TransferError::SameWarehouse)));
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
