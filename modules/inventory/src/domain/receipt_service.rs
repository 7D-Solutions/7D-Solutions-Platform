//! Atomic stock receipt service.
//!
//! Invariants:
//! - Ledger row + FIFO layer + outbox event created in a single transaction
//! - Idempotency key prevents double-processing on retry
//! - Guards run before any mutation
//!
//! Pattern: Guard → Mutation → Outbox (all in one transaction)

use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

use crate::{
    domain::guards::{GuardError, guard_cost_present, guard_item_active, guard_quantity_positive},
    events::{
        contracts::{ItemReceivedPayload, build_item_received_envelope},
        EVENT_TYPE_ITEM_RECEIVED,
    },
};

// ============================================================================
// Types
// ============================================================================

/// Input for POST /api/inventory/receipts
#[derive(Debug, Serialize, Deserialize)]
pub struct ReceiptRequest {
    pub tenant_id: String,
    pub item_id: Uuid,
    pub warehouse_id: Uuid,
    /// Quantity received (must be > 0)
    pub quantity: i64,
    /// Unit cost in minor currency units, e.g. cents (must be > 0)
    pub unit_cost_minor: i64,
    pub currency: String,
    pub purchase_order_id: Option<Uuid>,
    /// Caller-supplied idempotency key (required; scoped per tenant)
    pub idempotency_key: String,
    /// Distributed trace correlation ID (optional; generated if absent)
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
}

/// Result returned on successful or replayed receipt
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceiptResult {
    /// Stable business key for this receipt (from ledger.entry_id)
    pub receipt_line_id: Uuid,
    /// BIGSERIAL ledger row id (used for FIFO ordering)
    pub ledger_entry_id: i64,
    /// FIFO layer id
    pub layer_id: Uuid,
    /// Event id used in outbox (also = ledger.source_event_id)
    pub event_id: Uuid,
    pub tenant_id: String,
    pub item_id: Uuid,
    pub warehouse_id: Uuid,
    pub quantity: i64,
    pub unit_cost_minor: i64,
    pub currency: String,
    pub received_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Error)]
pub enum ReceiptError {
    #[error("Guard failed: {0}")]
    Guard(#[from] GuardError),

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
    entry_id: Uuid,
}

#[derive(sqlx::FromRow)]
struct IdempotencyRecord {
    response_body: String, // read as JSONB::TEXT
    request_hash: String,
}

// ============================================================================
// Service
// ============================================================================

/// Process a stock receipt atomically.
///
/// Returns `(ReceiptResult, is_replay)`.
/// - `is_replay = false`: new receipt created; HTTP 201.
/// - `is_replay = true`:  idempotency key matched; HTTP 200 with stored result.
pub async fn process_receipt(
    pool: &PgPool,
    req: &ReceiptRequest,
) -> Result<(ReceiptResult, bool), ReceiptError> {
    // --- Stateless input validation ---
    validate_request(req)?;

    // --- Compute request hash for idempotency conflict detection ---
    let request_hash = serde_json::to_string(req)?;

    // --- Idempotency check (read outside tx; fast path for replays) ---
    if let Some(record) = find_idempotency_key(pool, &req.tenant_id, &req.idempotency_key).await? {
        if record.request_hash != request_hash {
            return Err(ReceiptError::ConflictingIdempotencyKey);
        }
        let result: ReceiptResult = serde_json::from_str(&record.response_body)?;
        return Ok((result, true));
    }

    // --- DB guard: item must exist and be active ---
    let item = guard_item_active(pool, req.item_id, &req.tenant_id).await?;

    // --- Atomic transaction: ledger + FIFO layer + outbox + idempotency key ---
    let event_id = Uuid::new_v4();
    let received_at = Utc::now();
    let correlation_id = req
        .correlation_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let mut tx = pool.begin().await?;

    // Step 1: Insert ledger row
    let ledger_row = sqlx::query_as::<_, LedgerRow>(
        r#"
        INSERT INTO inventory_ledger
            (tenant_id, item_id, warehouse_id, entry_type, quantity,
             unit_cost_minor, currency, source_event_id, source_event_type,
             reference_type, reference_id, posted_at)
        VALUES
            ($1, $2, $3, 'received', $4, $5, $6, $7, $8, $9, $10, $11)
        RETURNING id, entry_id
        "#,
    )
    .bind(&req.tenant_id)
    .bind(req.item_id)
    .bind(req.warehouse_id)
    .bind(req.quantity)
    .bind(req.unit_cost_minor)
    .bind(&req.currency)
    .bind(event_id)
    .bind(EVENT_TYPE_ITEM_RECEIVED)
    .bind(req.purchase_order_id.map(|_| "purchase_order"))
    .bind(req.purchase_order_id.map(|id| id.to_string()))
    .bind(received_at)
    .fetch_one(&mut *tx)
    .await?;

    let ledger_entry_id = ledger_row.id;
    let receipt_line_id = ledger_row.entry_id;

    // Step 2: Insert FIFO layer
    let layer_id = sqlx::query_scalar::<_, Uuid>(
        r#"
        INSERT INTO inventory_layers
            (tenant_id, item_id, warehouse_id, ledger_entry_id, received_at,
             quantity_received, quantity_remaining, unit_cost_minor, currency)
        VALUES
            ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        RETURNING id
        "#,
    )
    .bind(&req.tenant_id)
    .bind(req.item_id)
    .bind(req.warehouse_id)
    .bind(ledger_entry_id)
    .bind(received_at)
    .bind(req.quantity) // quantity_received
    .bind(req.quantity) // quantity_remaining = quantity_received on insert
    .bind(req.unit_cost_minor)
    .bind(&req.currency)
    .fetch_one(&mut *tx)
    .await?;

    // Step 3: Build event envelope and enqueue in outbox
    let payload = ItemReceivedPayload {
        receipt_line_id,
        tenant_id: req.tenant_id.clone(),
        item_id: req.item_id,
        sku: item.sku,
        warehouse_id: req.warehouse_id,
        quantity: req.quantity,
        unit_cost_minor: req.unit_cost_minor,
        currency: req.currency.clone(),
        purchase_order_id: req.purchase_order_id,
        received_at,
    };

    let envelope = build_item_received_envelope(
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
    .bind(EVENT_TYPE_ITEM_RECEIVED)
    .bind(req.item_id.to_string())
    .bind(&req.tenant_id)
    .bind(&envelope_json)
    .bind(&correlation_id)
    .bind(&req.causation_id)
    .execute(&mut *tx)
    .await?;

    // Step 4: Build result
    let result = ReceiptResult {
        receipt_line_id,
        ledger_entry_id,
        layer_id,
        event_id,
        tenant_id: req.tenant_id.clone(),
        item_id: req.item_id,
        warehouse_id: req.warehouse_id,
        quantity: req.quantity,
        unit_cost_minor: req.unit_cost_minor,
        currency: req.currency.clone(),
        received_at,
    };

    // Step 5: Store idempotency key with response (expires in 7 days)
    let response_json = serde_json::to_string(&result)?;
    let expires_at = received_at + Duration::days(7);

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

fn validate_request(req: &ReceiptRequest) -> Result<(), ReceiptError> {
    if req.idempotency_key.trim().is_empty() {
        return Err(ReceiptError::Guard(GuardError::Validation(
            "idempotency_key is required".to_string(),
        )));
    }
    if req.tenant_id.trim().is_empty() {
        return Err(ReceiptError::Guard(GuardError::Validation(
            "tenant_id is required".to_string(),
        )));
    }
    if req.currency.trim().is_empty() {
        return Err(ReceiptError::Guard(GuardError::Validation(
            "currency is required".to_string(),
        )));
    }
    guard_quantity_positive(req.quantity)?;
    guard_cost_present(req.unit_cost_minor)?;
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
// Unit tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_req() -> ReceiptRequest {
        ReceiptRequest {
            tenant_id: "tenant-1".to_string(),
            item_id: Uuid::new_v4(),
            warehouse_id: Uuid::new_v4(),
            quantity: 10,
            unit_cost_minor: 5000,
            currency: "usd".to_string(),
            purchase_order_id: None,
            idempotency_key: "idem-001".to_string(),
            correlation_id: None,
            causation_id: None,
        }
    }

    #[test]
    fn validate_rejects_empty_idempotency_key() {
        let mut r = valid_req();
        r.idempotency_key = "  ".to_string();
        assert!(matches!(validate_request(&r), Err(ReceiptError::Guard(_))));
    }

    #[test]
    fn validate_rejects_empty_tenant() {
        let mut r = valid_req();
        r.tenant_id = "".to_string();
        assert!(matches!(validate_request(&r), Err(ReceiptError::Guard(_))));
    }

    #[test]
    fn validate_rejects_zero_quantity() {
        let mut r = valid_req();
        r.quantity = 0;
        assert!(matches!(validate_request(&r), Err(ReceiptError::Guard(_))));
    }

    #[test]
    fn validate_rejects_zero_cost() {
        let mut r = valid_req();
        r.unit_cost_minor = 0;
        assert!(matches!(validate_request(&r), Err(ReceiptError::Guard(_))));
    }

    #[test]
    fn validate_rejects_empty_currency() {
        let mut r = valid_req();
        r.currency = "".to_string();
        assert!(matches!(validate_request(&r), Err(ReceiptError::Guard(_))));
    }

    #[test]
    fn validate_accepts_valid_request() {
        assert!(validate_request(&valid_req()).is_ok());
    }
}
