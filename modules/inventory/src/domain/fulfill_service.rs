//! Reservation fulfillment service.
//!
//! Fulfillment converts an active reservation into a physical stock deduction.
//! Unlike a release (which returns stock to available), fulfillment reduces
//! quantity_on_hand — the stock physically left the warehouse.
//!
//! ## Lifecycle
//!   active reservation → fulfilled (compensating row, status='fulfilled')
//!
//! ## DB changes in one transaction:
//!   1. Insert compensating reservation row (status='fulfilled', reverses_reservation_id=<original>)
//!   2. Decrement quantity_reserved (hold consumed)
//!   3. Decrement quantity_on_hand (physical stock issued)
//!   4. Decrement available_status_on_hand (available stock issued)
//!   5. Write outbox event: inventory.reservation_fulfilled
//!   6. Store idempotency key
//!
//! Pattern: Guard → Mutation → Outbox (all in one transaction).

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

use crate::domain::guards::{GuardError, guard_quantity_positive};

const EVENT_TYPE_RESERVATION_FULFILLED: &str = "inventory.reservation_fulfilled";

// ============================================================================
// Types
// ============================================================================

/// Input for POST /api/inventory/reservations/{reservation_id}/fulfill
#[derive(Debug, Serialize, Deserialize)]
pub struct FulfillRequest {
    pub tenant_id: String,
    /// UUID of the original reserve row to fulfill
    pub reservation_id: Uuid,
    /// Optional partial fulfillment quantity (defaults to the full reserved quantity)
    pub quantity: Option<i64>,
    /// Business reference (e.g. fulfillment_order_id, order_ref)
    pub order_ref: Option<String>,
    /// Caller-supplied idempotency key (required; scoped per tenant)
    pub idempotency_key: String,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
}

/// Result returned on successful or replayed fulfillment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FulfillResult {
    /// UUID of the compensating fulfillment row
    pub fulfillment_id: Uuid,
    /// UUID of the original reservation that was fulfilled
    pub reservation_id: Uuid,
    /// UUID of the outbox event
    pub event_id: Uuid,
    pub tenant_id: String,
    pub item_id: Uuid,
    pub warehouse_id: Uuid,
    pub quantity: i64,
    pub order_ref: Option<String>,
    pub fulfilled_at: DateTime<Utc>,
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug, Error)]
pub enum FulfillError {
    #[error("Guard failed: {0}")]
    Guard(#[from] GuardError),

    #[error("Reservation not found")]
    ReservationNotFound,

    #[error("Reservation already released or fulfilled")]
    AlreadySettled,

    #[error("Fulfill quantity {0} exceeds reserved quantity {1}")]
    QuantityExceedsReserved(i64, i64),

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
struct IdempotencyRecord {
    response_body: String,
    request_hash: String,
}

#[derive(sqlx::FromRow)]
struct ReservationRow {
    id: Uuid,
    tenant_id: String,
    item_id: Uuid,
    warehouse_id: Uuid,
    quantity: i64,
}

// ============================================================================
// Fulfill
// ============================================================================

/// Fulfill a stock reservation atomically.
///
/// Converts the reservation hold into a physical stock deduction:
/// - Inserts a compensating row (status='fulfilled')
/// - Decrements quantity_reserved, quantity_on_hand, available_status_on_hand
/// - Writes outbox event: inventory.reservation_fulfilled
///
/// Returns `(FulfillResult, is_replay)`.
/// - `is_replay = false`: new fulfillment committed; HTTP 200.
/// - `is_replay = true`:  idempotency key matched; HTTP 200.
pub async fn process_fulfill(
    pool: &PgPool,
    req: &FulfillRequest,
) -> Result<(FulfillResult, bool), FulfillError> {
    validate_fulfill(req)?;

    let request_hash = serde_json::to_string(req)?;

    // Fast-path idempotency check.
    if let Some(record) = find_idempotency_key(pool, &req.tenant_id, &req.idempotency_key).await? {
        if record.request_hash != request_hash {
            return Err(FulfillError::ConflictingIdempotencyKey);
        }
        let result: FulfillResult = serde_json::from_str(&record.response_body)?;
        return Ok((result, true));
    }

    // Fetch the original reserve row — must be primary (reverses_reservation_id IS NULL)
    // and belong to the requesting tenant.
    let original = sqlx::query_as::<_, ReservationRow>(
        r#"
        SELECT id, tenant_id, item_id, warehouse_id, quantity
        FROM inventory_reservations
        WHERE id = $1 AND tenant_id = $2 AND reverses_reservation_id IS NULL
        "#,
    )
    .bind(req.reservation_id)
    .bind(&req.tenant_id)
    .fetch_optional(pool)
    .await?
    .ok_or(FulfillError::ReservationNotFound)?;

    // Guard: no compensating entry may already exist.
    let already_settled: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM inventory_reservations WHERE reverses_reservation_id = $1)",
    )
    .bind(original.id)
    .fetch_one(pool)
    .await?;

    if already_settled {
        return Err(FulfillError::AlreadySettled);
    }

    // Determine actual quantity to fulfill (default: full reserved quantity).
    let qty = req.quantity.unwrap_or(original.quantity);
    if qty > original.quantity {
        return Err(FulfillError::QuantityExceedsReserved(qty, original.quantity));
    }

    let event_id = Uuid::new_v4();
    let fulfilled_at = Utc::now();
    let correlation_id = req
        .correlation_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let mut tx = pool.begin().await?;

    // Step 1: Insert compensating reservation row (status='fulfilled').
    let fulfillment_id = sqlx::query_scalar::<_, Uuid>(
        r#"
        INSERT INTO inventory_reservations
            (tenant_id, item_id, warehouse_id, quantity, status,
             reverses_reservation_id, reference_id, fulfilled_at)
        VALUES
            ($1, $2, $3, $4, 'fulfilled', $5, $6, $7)
        RETURNING id
        "#,
    )
    .bind(&original.tenant_id)
    .bind(original.item_id)
    .bind(original.warehouse_id)
    .bind(qty)
    .bind(original.id)
    .bind(&req.order_ref)
    .bind(fulfilled_at)
    .fetch_one(&mut *tx)
    .await?;

    // Step 2: Update item_on_hand — decrement reserved AND on_hand simultaneously.
    //   quantity_reserved -= qty  (hold consumed)
    //   quantity_on_hand  -= qty  (physical stock issued)
    //   available_status_on_hand -= qty (available stock issued)
    //   quantity_available (generated) = available_status_on_hand - quantity_reserved → stays same
    sqlx::query(
        r#"
        UPDATE item_on_hand
        SET quantity_reserved        = GREATEST(0, quantity_reserved - $1),
            quantity_on_hand         = GREATEST(0, quantity_on_hand - $1),
            available_status_on_hand = GREATEST(0, available_status_on_hand - $1),
            projected_at             = NOW()
        WHERE tenant_id    = $2
          AND item_id      = $3
          AND warehouse_id = $4
          AND location_id IS NULL
        "#,
    )
    .bind(qty)
    .bind(&original.tenant_id)
    .bind(original.item_id)
    .bind(original.warehouse_id)
    .execute(&mut *tx)
    .await?;

    // Step 3: Write outbox event.
    let payload = serde_json::json!({
        "fulfillment_id":  fulfillment_id,
        "reservation_id":  original.id,
        "tenant_id":       original.tenant_id,
        "item_id":         original.item_id,
        "warehouse_id":    original.warehouse_id,
        "quantity":        qty,
        "order_ref":       req.order_ref,
        "fulfilled_at":    fulfilled_at,
    });

    sqlx::query(
        r#"
        INSERT INTO inv_outbox
            (event_id, event_type, aggregate_type, aggregate_id, tenant_id,
             payload, correlation_id, causation_id, schema_version)
        VALUES
            ($1, $2, 'inventory_reservation', $3, $4, $5::JSONB, $6, $7, '1.0.0')
        "#,
    )
    .bind(event_id)
    .bind(EVENT_TYPE_RESERVATION_FULFILLED)
    .bind(original.id.to_string())
    .bind(&original.tenant_id)
    .bind(payload.to_string())
    .bind(&correlation_id)
    .bind(&req.causation_id)
    .execute(&mut *tx)
    .await?;

    // Step 4: Build result.
    let result = FulfillResult {
        fulfillment_id,
        reservation_id: original.id,
        event_id,
        tenant_id: original.tenant_id.clone(),
        item_id: original.item_id,
        warehouse_id: original.warehouse_id,
        quantity: qty,
        order_ref: req.order_ref.clone(),
        fulfilled_at,
    };

    // Step 5: Store idempotency key (expires in 7 days).
    let response_json = serde_json::to_string(&result)?;
    let expires_at = fulfilled_at + Duration::days(7);

    sqlx::query(
        r#"
        INSERT INTO inv_idempotency_keys
            (tenant_id, idempotency_key, request_hash, response_body, status_code, expires_at)
        VALUES ($1, $2, $3, $4::JSONB, 200, $5)
        "#,
    )
    .bind(&original.tenant_id)
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

fn validate_fulfill(req: &FulfillRequest) -> Result<(), FulfillError> {
    if req.idempotency_key.trim().is_empty() {
        return Err(FulfillError::Guard(GuardError::Validation(
            "idempotency_key is required".to_string(),
        )));
    }
    if req.tenant_id.trim().is_empty() {
        return Err(FulfillError::Guard(GuardError::Validation(
            "tenant_id is required".to_string(),
        )));
    }
    if let Some(qty) = req.quantity {
        guard_quantity_positive(qty)?;
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

// ============================================================================
// Unit tests (stateless validation only)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_fulfill() -> FulfillRequest {
        FulfillRequest {
            tenant_id: "tenant-1".to_string(),
            reservation_id: Uuid::new_v4(),
            quantity: None,
            order_ref: Some("order-123".to_string()),
            idempotency_key: "idem-fulfill-001".to_string(),
            correlation_id: None,
            causation_id: None,
        }
    }

    #[test]
    fn fulfill_rejects_empty_idempotency_key() {
        let mut r = valid_fulfill();
        r.idempotency_key = "".to_string();
        assert!(matches!(validate_fulfill(&r), Err(FulfillError::Guard(_))));
    }

    #[test]
    fn fulfill_rejects_empty_tenant() {
        let mut r = valid_fulfill();
        r.tenant_id = "".to_string();
        assert!(matches!(validate_fulfill(&r), Err(FulfillError::Guard(_))));
    }

    #[test]
    fn fulfill_rejects_zero_quantity() {
        let mut r = valid_fulfill();
        r.quantity = Some(0);
        assert!(matches!(validate_fulfill(&r), Err(FulfillError::Guard(_))));
    }

    #[test]
    fn fulfill_accepts_valid_request() {
        assert!(validate_fulfill(&valid_fulfill()).is_ok());
    }

    #[test]
    fn fulfill_accepts_none_quantity_uses_full_reserved() {
        let mut r = valid_fulfill();
        r.quantity = None;
        assert!(validate_fulfill(&r).is_ok());
    }
}
