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
use utoipa::ToSchema;
use uuid::Uuid;

use crate::domain::guards::{guard_item_active, guard_quantity_positive, GuardError};
use crate::domain::reservation_repo;

// Internal event type strings — not shared with external consumers yet.
const EVENT_TYPE_ITEM_RESERVED: &str = "inventory.item_reserved";
const EVENT_TYPE_RESERVATION_RELEASED: &str = "inventory.reservation_released";

// ============================================================================
// Types: Reserve
// ============================================================================

/// Input for POST /api/inventory/reservations/reserve
#[derive(Debug, Serialize, Deserialize, ToSchema)]
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
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
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
#[derive(Debug, Serialize, Deserialize, ToSchema)]
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
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
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

    #[error("Insufficient available stock: requested {requested}, available {available}")]
    InsufficientAvailable { requested: i64, available: i64 },

    #[error("Idempotency key conflict: same key used with a different request body")]
    ConflictingIdempotencyKey,

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
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
    if let Some(record) =
        reservation_repo::find_idempotency_key(pool, &req.tenant_id, &req.idempotency_key).await?
    {
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

    // Step 0: Lock item_on_hand row and check available stock.
    //   SELECT FOR UPDATE serializes concurrent reservations at the row lock.
    //   quantity_available = quantity_on_hand - quantity_reserved (generated column).
    let available: Option<i64> =
        reservation_repo::lock_available_stock(&mut tx, &req.tenant_id, req.item_id, req.warehouse_id)
            .await?;

    let current_available = available.unwrap_or(0);
    if current_available < req.quantity {
        return Err(ReservationError::InsufficientAvailable {
            requested: req.quantity,
            available: current_available,
        });
    }

    // Step 1: Insert primary reservation row.
    //   status = 'active', reverses_reservation_id = NULL (primary entry).
    let reservation_id = reservation_repo::insert_reservation(
        &mut tx,
        &req.tenant_id,
        req.item_id,
        req.warehouse_id,
        req.quantity,
        req.reference_type.as_deref(),
        req.reference_id.as_deref(),
        reserved_at,
        req.expires_at,
    )
    .await?;

    // Step 2: Upsert on-hand projection — increment quantity_reserved.
    //   Reservations operate on the null-location row (warehouse-level) in v1.
    //   quantity_available is GENERATED ALWAYS AS (available_status_on_hand - reserved).
    reservation_repo::increment_reserved(
        &mut tx,
        &req.tenant_id,
        req.item_id,
        req.warehouse_id,
        req.quantity,
    )
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

    reservation_repo::insert_outbox_event(
        &mut tx,
        event_id,
        EVENT_TYPE_ITEM_RESERVED,
        &reservation_id.to_string(),
        &req.tenant_id,
        &payload.to_string(),
        &correlation_id,
        req.causation_id.as_deref(),
    )
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

    reservation_repo::store_idempotency_key(
        &mut tx,
        &req.tenant_id,
        &req.idempotency_key,
        &request_hash,
        &response_json,
        201,
        expires_at,
    )
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
    if let Some(record) =
        reservation_repo::find_idempotency_key(pool, &req.tenant_id, &req.idempotency_key).await?
    {
        if record.request_hash != request_hash {
            return Err(ReservationError::ConflictingIdempotencyKey);
        }
        let result: ReleaseResult = serde_json::from_str(&record.response_body)?;
        return Ok((result, true));
    }

    // Fetch the original reserve row — must be a primary entry (reverses_reservation_id IS NULL)
    // and belong to the requesting tenant.
    let original =
        reservation_repo::find_active_reservation(pool, req.reservation_id, &req.tenant_id)
            .await?
            .ok_or(ReservationError::ReservationNotFound)?;

    // Guard: no compensating entry may already exist.
    if reservation_repo::is_already_compensated(pool, original.id).await? {
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
    let release_id =
        reservation_repo::insert_compensating_reservation(&mut tx, &original, released_at)
            .await?;

    // Step 2: Decrement quantity_reserved on the null-location on-hand row.
    //   Reservations are warehouse-level in v1 (location_id IS NULL).
    //   GREATEST(0, ...) is a safety floor in case of projection skew.
    reservation_repo::decrement_reserved(&mut tx, &original).await?;

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

    reservation_repo::insert_outbox_event(
        &mut tx,
        event_id,
        EVENT_TYPE_RESERVATION_RELEASED,
        &original.id.to_string(),
        &original.tenant_id,
        &payload.to_string(),
        &correlation_id,
        req.causation_id.as_deref(),
    )
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

    reservation_repo::store_idempotency_key(
        &mut tx,
        &original.tenant_id,
        &req.idempotency_key,
        &request_hash,
        &response_json,
        200,
        expires_at,
    )
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
        assert!(matches!(
            validate_reserve(&r),
            Err(ReservationError::Guard(_))
        ));
    }

    #[test]
    fn reserve_rejects_empty_tenant() {
        let mut r = valid_reserve();
        r.tenant_id = "".to_string();
        assert!(matches!(
            validate_reserve(&r),
            Err(ReservationError::Guard(_))
        ));
    }

    #[test]
    fn reserve_rejects_zero_quantity() {
        let mut r = valid_reserve();
        r.quantity = 0;
        assert!(matches!(
            validate_reserve(&r),
            Err(ReservationError::Guard(_))
        ));
    }

    #[test]
    fn reserve_rejects_negative_quantity() {
        let mut r = valid_reserve();
        r.quantity = -5;
        assert!(matches!(
            validate_reserve(&r),
            Err(ReservationError::Guard(_))
        ));
    }

    #[test]
    fn reserve_accepts_valid_request() {
        assert!(validate_reserve(&valid_reserve()).is_ok());
    }

    #[test]
    fn release_rejects_empty_idempotency_key() {
        let mut r = valid_release();
        r.idempotency_key = "".to_string();
        assert!(matches!(
            validate_release(&r),
            Err(ReservationError::Guard(_))
        ));
    }

    #[test]
    fn release_rejects_empty_tenant() {
        let mut r = valid_release();
        r.tenant_id = "".to_string();
        assert!(matches!(
            validate_release(&r),
            Err(ReservationError::Guard(_))
        ));
    }

    #[test]
    fn release_accepts_valid_request() {
        assert!(validate_release(&valid_release()).is_ok());
    }
}
