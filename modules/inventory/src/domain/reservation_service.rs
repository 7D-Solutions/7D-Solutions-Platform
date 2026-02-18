//! Reservation service: reserve + release (compensating entry model).
//!
//! Invariants:
//! - Reserve creates an append-only reservation row (status='active',
//!   reverses_reservation_id=NULL) and increments quantity_reserved on the
//!   on-hand projection, all in a single transaction.
//! - Release creates a compensating row (status='released',
//!   reverses_reservation_id=<original id>) and decrements quantity_reserved,
//!   all in a single transaction. Original rows are NEVER mutated.
//! - Both operations are idempotent via inv_idempotency_keys.
//! - Outbox event written in the same transaction for downstream consumers.
//! - Available = quantity_on_hand - quantity_reserved (generated column in DB).
//!
//! Pattern: Guard → Mutation → Outbox (all in one transaction).

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

use crate::domain::guards::{GuardError, guard_item_active, guard_quantity_positive};

// Internal event type strings — not shared with external consumers yet.
const EVENT_TYPE_ITEM_RESERVED: &str = "inventory.item_reserved";
const EVENT_TYPE_RESERVATION_RELEASED: &str = "inventory.reservation_released";

// ============================================================================
// Types: Reserve
// ============================================================================

/// Input for POST /api/inventory/reservations/reserve
#[derive(Debug, Serialize, Deserialize)]
pub struct ReserveRequest {
    pub tenant_id: String,
    pub item_id: Uuid,
    pub warehouse_id: Uuid,
    /// Units to hold (must be > 0)
    pub quantity: i64,
    /// Optional business reference (e.g. "sales_order", "fulfillment_order")
    pub reference_type: Option<String>,
    pub reference_id: Option<String>,
    /// Optional TTL for the hold
    pub expires_at: Option<DateTime<Utc>>,
    /// Caller-supplied idempotency key (required; scoped per tenant)
    pub idempotency_key: String,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
}

/// Result returned on successful or replayed reserve
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReserveResult {
    /// UUID of the created reservation row (primary entry)
    pub reservation_id: Uuid,
    /// UUID of the outbox event
    pub event_id: Uuid,
    pub tenant_id: String,
    pub item_id: Uuid,
    pub warehouse_id: Uuid,
    pub quantity: i64,
    pub reserved_at: DateTime<Utc>,
}

// ============================================================================
// Types: Release
// ============================================================================

/// Input for POST /api/inventory/reservations/release
#[derive(Debug, Serialize, Deserialize)]
pub struct ReleaseRequest {
    pub tenant_id: String,
    /// UUID of the original reserve row to compensate
    pub reservation_id: Uuid,
    /// Caller-supplied idempotency key (required; scoped per tenant)
    pub idempotency_key: String,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
}

/// Result returned on successful or replayed release
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseResult {
    /// UUID of the compensating reservation row (release entry)
    pub release_id: Uuid,
    /// UUID of the original reservation that was released
    pub reservation_id: Uuid,
    /// UUID of the outbox event
    pub event_id: Uuid,
    pub tenant_id: String,
    pub released_at: DateTime<Utc>,
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug, Error)]
pub enum ReservationError {
    #[error("Guard failed: {0}")]
    Guard(#[from] GuardError),

    #[error("Reservation not found")]
    ReservationNotFound,

    #[error("Reservation already released or fulfilled")]
    AlreadyReleased,

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

/// Minimal reservation row fetched for the release guard check.
#[derive(sqlx::FromRow)]
struct ReservationRow {
    id: Uuid,
    tenant_id: String,
    item_id: Uuid,
    warehouse_id: Uuid,
    quantity: i64,
}

// ============================================================================
// Reserve
// ============================================================================

/// Create a stock reservation atomically.
///
/// Returns `(ReserveResult, is_replay)`.
/// - `is_replay = false`: new reservation; HTTP 201.
/// - `is_replay = true`:  idempotency key matched; HTTP 200.
pub async fn process_reserve(
    pool: &PgPool,
    req: &ReserveRequest,
) -> Result<(ReserveResult, bool), ReservationError> {
    validate_reserve(req)?;

    let request_hash = serde_json::to_string(req)?;

    // Fast-path: check idempotency key before hitting the DB further.
    if let Some(record) = find_idempotency_key(pool, &req.tenant_id, &req.idempotency_key).await? {
        if record.request_hash != request_hash {
            return Err(ReservationError::ConflictingIdempotencyKey);
        }
        let result: ReserveResult = serde_json::from_str(&record.response_body)?;
        return Ok((result, true));
    }

    // Guard: item must exist and be active for this tenant.
    guard_item_active(pool, req.item_id, &req.tenant_id).await?;

    let event_id = Uuid::new_v4();
    let reserved_at = Utc::now();
    let correlation_id = req
        .correlation_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let mut tx = pool.begin().await?;

    // Step 1: Insert primary reservation row.
    //   status = 'active', reverses_reservation_id = NULL (primary entry).
    let reservation_id = sqlx::query_scalar::<_, Uuid>(
        r#"
        INSERT INTO inventory_reservations
            (tenant_id, item_id, warehouse_id, quantity, status,
             reverses_reservation_id, reference_type, reference_id,
             reserved_at, expires_at)
        VALUES
            ($1, $2, $3, $4, 'active', NULL, $5, $6, $7, $8)
        RETURNING id
        "#,
    )
    .bind(&req.tenant_id)
    .bind(req.item_id)
    .bind(req.warehouse_id)
    .bind(req.quantity)
    .bind(&req.reference_type)
    .bind(&req.reference_id)
    .bind(reserved_at)
    .bind(req.expires_at)
    .fetch_one(&mut *tx)
    .await?;

    // Step 2: Upsert on-hand projection — increment quantity_reserved.
    //   quantity_available is GENERATED ALWAYS AS (on_hand - reserved), so only
    //   quantity_reserved is written here.
    sqlx::query(
        r#"
        INSERT INTO item_on_hand
            (tenant_id, item_id, warehouse_id, quantity_reserved, projected_at)
        VALUES ($1, $2, $3, $4, NOW())
        ON CONFLICT (tenant_id, item_id, warehouse_id) DO UPDATE
            SET quantity_reserved = item_on_hand.quantity_reserved + EXCLUDED.quantity_reserved,
                projected_at = NOW()
        "#,
    )
    .bind(&req.tenant_id)
    .bind(req.item_id)
    .bind(req.warehouse_id)
    .bind(req.quantity)
    .execute(&mut *tx)
    .await?;

    // Step 3: Write outbox event.
    let payload = serde_json::json!({
        "reservation_id": reservation_id,
        "tenant_id":      req.tenant_id,
        "item_id":        req.item_id,
        "warehouse_id":   req.warehouse_id,
        "quantity":       req.quantity,
        "reference_type": req.reference_type,
        "reference_id":   req.reference_id,
        "reserved_at":    reserved_at,
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
    .bind(EVENT_TYPE_ITEM_RESERVED)
    .bind(reservation_id.to_string())
    .bind(&req.tenant_id)
    .bind(payload.to_string())
    .bind(&correlation_id)
    .bind(&req.causation_id)
    .execute(&mut *tx)
    .await?;

    // Step 4: Build result.
    let result = ReserveResult {
        reservation_id,
        event_id,
        tenant_id: req.tenant_id.clone(),
        item_id: req.item_id,
        warehouse_id: req.warehouse_id,
        quantity: req.quantity,
        reserved_at,
    };

    // Step 5: Store idempotency key (expires in 7 days).
    let response_json = serde_json::to_string(&result)?;
    let expires_at = reserved_at + Duration::days(7);

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
// Release
// ============================================================================

/// Release a stock reservation atomically (compensating entry model).
///
/// Creates a new row with status='released' referencing the original reserve row.
/// The original row is never mutated.
///
/// Returns `(ReleaseResult, is_replay)`.
/// - `is_replay = false`: compensating entry committed; HTTP 200.
/// - `is_replay = true`:  idempotency key matched; HTTP 200.
pub async fn process_release(
    pool: &PgPool,
    req: &ReleaseRequest,
) -> Result<(ReleaseResult, bool), ReservationError> {
    validate_release(req)?;

    let request_hash = serde_json::to_string(req)?;

    // Fast-path idempotency check.
    if let Some(record) = find_idempotency_key(pool, &req.tenant_id, &req.idempotency_key).await? {
        if record.request_hash != request_hash {
            return Err(ReservationError::ConflictingIdempotencyKey);
        }
        let result: ReleaseResult = serde_json::from_str(&record.response_body)?;
        return Ok((result, true));
    }

    // Fetch the original reserve row — must be a primary entry (reverses_reservation_id IS NULL)
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
    .ok_or(ReservationError::ReservationNotFound)?;

    // Guard: no compensating entry may already exist.
    let already_compensated: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM inventory_reservations WHERE reverses_reservation_id = $1)",
    )
    .bind(original.id)
    .fetch_one(pool)
    .await?;

    if already_compensated {
        return Err(ReservationError::AlreadyReleased);
    }

    let event_id = Uuid::new_v4();
    let released_at = Utc::now();
    let correlation_id = req
        .correlation_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let mut tx = pool.begin().await?;

    // Step 1: Insert compensating reservation row.
    //   status = 'released', reverses_reservation_id = original.id.
    let release_id = sqlx::query_scalar::<_, Uuid>(
        r#"
        INSERT INTO inventory_reservations
            (tenant_id, item_id, warehouse_id, quantity, status,
             reverses_reservation_id, released_at)
        VALUES
            ($1, $2, $3, $4, 'released', $5, $6)
        RETURNING id
        "#,
    )
    .bind(&original.tenant_id)
    .bind(original.item_id)
    .bind(original.warehouse_id)
    .bind(original.quantity)
    .bind(original.id)
    .bind(released_at)
    .fetch_one(&mut *tx)
    .await?;

    // Step 2: Decrement quantity_reserved on the on-hand projection.
    //   GREATEST(0, ...) is a safety floor in case of projection skew; correct
    //   data will never go below zero here.
    sqlx::query(
        r#"
        UPDATE item_on_hand
        SET quantity_reserved = GREATEST(0, quantity_reserved - $1),
            projected_at = NOW()
        WHERE tenant_id = $2 AND item_id = $3 AND warehouse_id = $4
        "#,
    )
    .bind(original.quantity)
    .bind(&original.tenant_id)
    .bind(original.item_id)
    .bind(original.warehouse_id)
    .execute(&mut *tx)
    .await?;

    // Step 3: Write outbox event.
    let payload = serde_json::json!({
        "release_id":     release_id,
        "reservation_id": original.id,
        "tenant_id":      original.tenant_id,
        "item_id":        original.item_id,
        "warehouse_id":   original.warehouse_id,
        "quantity":       original.quantity,
        "released_at":    released_at,
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
    .bind(EVENT_TYPE_RESERVATION_RELEASED)
    .bind(original.id.to_string())
    .bind(&original.tenant_id)
    .bind(payload.to_string())
    .bind(&correlation_id)
    .bind(&req.causation_id)
    .execute(&mut *tx)
    .await?;

    // Step 4: Build result.
    let result = ReleaseResult {
        release_id,
        reservation_id: original.id,
        event_id,
        tenant_id: original.tenant_id.clone(),
        released_at,
    };

    // Step 5: Store idempotency key (expires in 7 days).
    let response_json = serde_json::to_string(&result)?;
    let expires_at = released_at + Duration::days(7);

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

fn validate_reserve(req: &ReserveRequest) -> Result<(), ReservationError> {
    if req.idempotency_key.trim().is_empty() {
        return Err(ReservationError::Guard(GuardError::Validation(
            "idempotency_key is required".to_string(),
        )));
    }
    if req.tenant_id.trim().is_empty() {
        return Err(ReservationError::Guard(GuardError::Validation(
            "tenant_id is required".to_string(),
        )));
    }
    guard_quantity_positive(req.quantity)?;
    Ok(())
}

fn validate_release(req: &ReleaseRequest) -> Result<(), ReservationError> {
    if req.idempotency_key.trim().is_empty() {
        return Err(ReservationError::Guard(GuardError::Validation(
            "idempotency_key is required".to_string(),
        )));
    }
    if req.tenant_id.trim().is_empty() {
        return Err(ReservationError::Guard(GuardError::Validation(
            "tenant_id is required".to_string(),
        )));
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

    fn valid_reserve() -> ReserveRequest {
        ReserveRequest {
            tenant_id: "tenant-1".to_string(),
            item_id: Uuid::new_v4(),
            warehouse_id: Uuid::new_v4(),
            quantity: 10,
            reference_type: None,
            reference_id: None,
            expires_at: None,
            idempotency_key: "idem-001".to_string(),
            correlation_id: None,
            causation_id: None,
        }
    }

    fn valid_release() -> ReleaseRequest {
        ReleaseRequest {
            tenant_id: "tenant-1".to_string(),
            reservation_id: Uuid::new_v4(),
            idempotency_key: "idem-002".to_string(),
            correlation_id: None,
            causation_id: None,
        }
    }

    #[test]
    fn reserve_rejects_empty_idempotency_key() {
        let mut r = valid_reserve();
        r.idempotency_key = "  ".to_string();
        assert!(matches!(validate_reserve(&r), Err(ReservationError::Guard(_))));
    }

    #[test]
    fn reserve_rejects_empty_tenant() {
        let mut r = valid_reserve();
        r.tenant_id = "".to_string();
        assert!(matches!(validate_reserve(&r), Err(ReservationError::Guard(_))));
    }

    #[test]
    fn reserve_rejects_zero_quantity() {
        let mut r = valid_reserve();
        r.quantity = 0;
        assert!(matches!(validate_reserve(&r), Err(ReservationError::Guard(_))));
    }

    #[test]
    fn reserve_rejects_negative_quantity() {
        let mut r = valid_reserve();
        r.quantity = -5;
        assert!(matches!(validate_reserve(&r), Err(ReservationError::Guard(_))));
    }

    #[test]
    fn reserve_accepts_valid_request() {
        assert!(validate_reserve(&valid_reserve()).is_ok());
    }

    #[test]
    fn release_rejects_empty_idempotency_key() {
        let mut r = valid_release();
        r.idempotency_key = "".to_string();
        assert!(matches!(validate_release(&r), Err(ReservationError::Guard(_))));
    }

    #[test]
    fn release_rejects_empty_tenant() {
        let mut r = valid_release();
        r.tenant_id = "".to_string();
        assert!(matches!(validate_release(&r), Err(ReservationError::Guard(_))));
    }

    #[test]
    fn release_accepts_valid_request() {
        assert!(validate_release(&valid_release()).is_ok());
    }
}
