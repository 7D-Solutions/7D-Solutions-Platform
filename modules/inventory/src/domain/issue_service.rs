//! Atomic stock issue service.
//!
//! Invariants:
//! - Lock per (tenant_id, item_id, warehouse_id) via SELECT … FOR UPDATE on FIFO layers.
//! - Available = sum(layer.quantity_remaining) − quantity_reserved (no negatives allowed).
//! - Deterministic FIFO consumption: oldest layer first, tie-break by ledger_entry_id.
//! - Ledger row + layer_consumptions + layer updates + on-hand projection + outbox event
//!   created in a single transaction.
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
    },
    events::{
        contracts::{ConsumedLayer, ItemIssuedPayload, SourceRef, build_item_issued_envelope},
        EVENT_TYPE_ITEM_ISSUED,
    },
};

// ============================================================================
// Types
// ============================================================================

/// Input for POST /api/inventory/issues
#[derive(Debug, Serialize, Deserialize)]
pub struct IssueRequest {
    pub tenant_id: String,
    pub item_id: Uuid,
    pub warehouse_id: Uuid,
    /// Quantity to issue (must be > 0)
    pub quantity: i64,
    pub currency: String,
    // Source reference (maps to SourceRef in event payload)
    pub source_module: String,
    pub source_type: String,
    pub source_id: String,
    pub source_line_id: Option<String>,
    /// Caller-supplied idempotency key (required; scoped per tenant)
    pub idempotency_key: String,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
}

/// Result returned on successful or replayed issue
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueResult {
    /// Stable business key for this issue (= ledger entry_id)
    pub issue_line_id: Uuid,
    /// BIGSERIAL ledger row id
    pub ledger_entry_id: i64,
    /// Event id used in outbox
    pub event_id: Uuid,
    pub tenant_id: String,
    pub item_id: Uuid,
    pub warehouse_id: Uuid,
    pub quantity: i64,
    pub total_cost_minor: i64,
    pub currency: String,
    pub consumed_layers: Vec<ConsumedLayer>,
    pub source_ref: SourceRef,
    pub issued_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Error)]
pub enum IssueError {
    #[error("Guard failed: {0}")]
    Guard(#[from] GuardError),

    #[error("Insufficient stock: requested {requested}, available {available}")]
    InsufficientQuantity { requested: i64, available: i64 },

    #[error("FIFO engine error: {0}")]
    Fifo(#[from] FifoError),

    #[error("No stock layers found for this item/warehouse")]
    NoLayersAvailable,

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

/// Process a stock issue atomically.
///
/// Returns `(IssueResult, is_replay)`.
/// - `is_replay = false`: new issue created; HTTP 201.
/// - `is_replay = true`:  idempotency key matched; HTTP 200.
pub async fn process_issue(
    pool: &PgPool,
    req: &IssueRequest,
) -> Result<(IssueResult, bool), IssueError> {
    // --- Stateless input validation ---
    validate_request(req)?;

    let request_hash = serde_json::to_string(req)?;

    // --- Idempotency fast-path ---
    if let Some(record) = find_idempotency_key(pool, &req.tenant_id, &req.idempotency_key).await? {
        if record.request_hash != request_hash {
            return Err(IssueError::ConflictingIdempotencyKey);
        }
        let result: IssueResult = serde_json::from_str(&record.response_body)?;
        return Ok((result, true));
    }

    // --- Guard: item must exist and be active ---
    let item = guard_item_active(pool, req.item_id, &req.tenant_id).await?;

    let event_id = Uuid::new_v4();
    let issued_at = Utc::now();
    let correlation_id = req
        .correlation_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let mut tx = pool.begin().await?;

    // --- Lock FIFO layers and read available stock ---
    // ORDER BY received_at ASC, ledger_entry_id ASC: deterministic FIFO.
    // FOR UPDATE: serialises concurrent issues for the same (tenant, item, warehouse).
    let layer_rows = sqlx::query_as::<_, LayerRow>(
        r#"
        SELECT id, quantity_remaining, unit_cost_minor
        FROM inventory_layers
        WHERE tenant_id = $1
          AND item_id   = $2
          AND warehouse_id = $3
          AND quantity_remaining > 0
        ORDER BY received_at ASC, ledger_entry_id ASC
        FOR UPDATE
        "#,
    )
    .bind(&req.tenant_id)
    .bind(req.item_id)
    .bind(req.warehouse_id)
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

    // Read reserved quantity from on-hand projection (may not exist yet → 0).
    let quantity_reserved: i64 = sqlx::query_scalar(
        r#"
        SELECT COALESCE(quantity_reserved, 0)
        FROM item_on_hand
        WHERE tenant_id = $1 AND item_id = $2 AND warehouse_id = $3
        "#,
    )
    .bind(&req.tenant_id)
    .bind(req.item_id)
    .bind(req.warehouse_id)
    .fetch_optional(&mut *tx)
    .await?
    .unwrap_or(0i64);

    let net_available = sum_remaining - quantity_reserved;

    if net_available < req.quantity {
        return Err(IssueError::InsufficientQuantity {
            requested: req.quantity,
            available: net_available,
        });
    }

    // --- Deterministic FIFO consumption ---
    let consumed = fifo::consume_fifo(&available_layers, req.quantity)?;
    let total_cost_minor: i64 = consumed.iter().map(|c| c.extended_cost_minor).sum();

    // Pre-issue total cost for on-hand projection update.
    let pre_issue_total_cost: i64 = available_layers
        .iter()
        .map(|l| l.quantity_remaining * l.unit_cost_minor)
        .sum();
    let post_issue_total_cost = (pre_issue_total_cost - total_cost_minor).max(0);
    let new_on_hand = sum_remaining - req.quantity;

    // --- Step 1: Insert ledger row (negative quantity = stock out) ---
    let ledger_row = sqlx::query_as::<_, LedgerRow>(
        r#"
        INSERT INTO inventory_ledger
            (tenant_id, item_id, warehouse_id, entry_type, quantity,
             unit_cost_minor, currency, source_event_id, source_event_type,
             reference_type, reference_id, posted_at)
        VALUES
            ($1, $2, $3, 'issued', $4, 0, $5, $6, $7, $8, $9, $10)
        RETURNING id, entry_id
        "#,
    )
    .bind(&req.tenant_id)
    .bind(req.item_id)
    .bind(req.warehouse_id)
    .bind(-req.quantity) // signed: negative = stock out
    .bind(&req.currency)
    .bind(event_id)
    .bind(EVENT_TYPE_ITEM_ISSUED)
    .bind(&req.source_type)
    .bind(&req.source_id)
    .bind(issued_at)
    .fetch_one(&mut *tx)
    .await?;

    let ledger_id = ledger_row.id;
    let issue_line_id = ledger_row.entry_id;

    // --- Step 2: Insert layer_consumptions + update layer quantity_remaining ---
    for c in &consumed {
        sqlx::query(
            r#"
            INSERT INTO layer_consumptions
                (layer_id, ledger_entry_id, quantity_consumed, unit_cost_minor, consumed_at)
            VALUES ($1, $2, $3, $4, $5)
            "#,
        )
        .bind(c.layer_id)
        .bind(ledger_id)
        .bind(c.quantity)
        .bind(c.unit_cost_minor)
        .bind(issued_at)
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
        .bind(issued_at)
        .bind(c.layer_id)
        .execute(&mut *tx)
        .await?;
    }

    // --- Step 3: Upsert on-hand projection ---
    sqlx::query(
        r#"
        INSERT INTO item_on_hand
            (tenant_id, item_id, warehouse_id, quantity_on_hand,
             total_cost_minor, currency, last_ledger_entry_id, projected_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, NOW())
        ON CONFLICT (tenant_id, item_id, warehouse_id) DO UPDATE
            SET quantity_on_hand      = $4,
                total_cost_minor      = $5,
                last_ledger_entry_id  = $7,
                projected_at          = NOW()
        "#,
    )
    .bind(&req.tenant_id)
    .bind(req.item_id)
    .bind(req.warehouse_id)
    .bind(new_on_hand)
    .bind(post_issue_total_cost)
    .bind(&req.currency)
    .bind(ledger_id)
    .execute(&mut *tx)
    .await?;

    // --- Step 4: Build and enqueue outbox event ---
    let source_ref = SourceRef {
        source_module: req.source_module.clone(),
        source_type: req.source_type.clone(),
        source_id: req.source_id.clone(),
        source_line_id: req.source_line_id.clone(),
    };

    let payload = ItemIssuedPayload {
        issue_line_id,
        tenant_id: req.tenant_id.clone(),
        item_id: req.item_id,
        sku: item.sku,
        warehouse_id: req.warehouse_id,
        quantity: req.quantity,
        total_cost_minor,
        currency: req.currency.clone(),
        consumed_layers: consumed.clone(),
        source_ref: source_ref.clone(),
        issued_at,
    };

    let envelope = build_item_issued_envelope(
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
    .bind(EVENT_TYPE_ITEM_ISSUED)
    .bind(req.item_id.to_string())
    .bind(&req.tenant_id)
    .bind(&envelope_json)
    .bind(&correlation_id)
    .bind(&req.causation_id)
    .execute(&mut *tx)
    .await?;

    // --- Step 5: Build result ---
    let result = IssueResult {
        issue_line_id,
        ledger_entry_id: ledger_id,
        event_id,
        tenant_id: req.tenant_id.clone(),
        item_id: req.item_id,
        warehouse_id: req.warehouse_id,
        quantity: req.quantity,
        total_cost_minor,
        currency: req.currency.clone(),
        consumed_layers: consumed,
        source_ref,
        issued_at,
    };

    // --- Step 6: Store idempotency key (expires in 7 days) ---
    let response_json = serde_json::to_string(&result)?;
    let expires_at = issued_at + Duration::days(7);

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

fn validate_request(req: &IssueRequest) -> Result<(), IssueError> {
    if req.idempotency_key.trim().is_empty() {
        return Err(IssueError::Guard(GuardError::Validation(
            "idempotency_key is required".to_string(),
        )));
    }
    if req.tenant_id.trim().is_empty() {
        return Err(IssueError::Guard(GuardError::Validation(
            "tenant_id is required".to_string(),
        )));
    }
    if req.currency.trim().is_empty() {
        return Err(IssueError::Guard(GuardError::Validation(
            "currency is required".to_string(),
        )));
    }
    if req.source_module.trim().is_empty() || req.source_type.trim().is_empty() || req.source_id.trim().is_empty() {
        return Err(IssueError::Guard(GuardError::Validation(
            "source_module, source_type, and source_id are required".to_string(),
        )));
    }
    guard_quantity_positive(req.quantity)?;
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

    fn valid_req() -> IssueRequest {
        IssueRequest {
            tenant_id: "tenant-1".to_string(),
            item_id: Uuid::new_v4(),
            warehouse_id: Uuid::new_v4(),
            quantity: 5,
            currency: "usd".to_string(),
            source_module: "orders".to_string(),
            source_type: "sales_order".to_string(),
            source_id: "SO-001".to_string(),
            source_line_id: None,
            idempotency_key: "idem-001".to_string(),
            correlation_id: None,
            causation_id: None,
        }
    }

    #[test]
    fn validate_rejects_empty_idempotency_key() {
        let mut r = valid_req();
        r.idempotency_key = "  ".to_string();
        assert!(matches!(validate_request(&r), Err(IssueError::Guard(_))));
    }

    #[test]
    fn validate_rejects_empty_tenant() {
        let mut r = valid_req();
        r.tenant_id = "".to_string();
        assert!(matches!(validate_request(&r), Err(IssueError::Guard(_))));
    }

    #[test]
    fn validate_rejects_zero_quantity() {
        let mut r = valid_req();
        r.quantity = 0;
        assert!(matches!(validate_request(&r), Err(IssueError::Guard(_))));
    }

    #[test]
    fn validate_rejects_negative_quantity() {
        let mut r = valid_req();
        r.quantity = -1;
        assert!(matches!(validate_request(&r), Err(IssueError::Guard(_))));
    }

    #[test]
    fn validate_rejects_empty_currency() {
        let mut r = valid_req();
        r.currency = "".to_string();
        assert!(matches!(validate_request(&r), Err(IssueError::Guard(_))));
    }

    #[test]
    fn validate_rejects_empty_source_id() {
        let mut r = valid_req();
        r.source_id = "".to_string();
        assert!(matches!(validate_request(&r), Err(IssueError::Guard(_))));
    }

    #[test]
    fn validate_accepts_valid_request() {
        assert!(validate_request(&valid_req()).is_ok());
    }
}
