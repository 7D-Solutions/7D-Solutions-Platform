//! Status bucket transfer service (Guard → Mutation → Outbox atomicity).
//!
//! Moves quantity between status buckets (available | quarantine | damaged).
//! Available transfers guard against reserved stock (uses quantity_available).
//! Idempotency via `inv_idempotency_keys`. Append-only ledger in `inv_status_transfers`.

use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    domain::guards::{guard_item_active, guard_quantity_positive, GuardError},
    domain::status::{models::InvItemStatus, repo},
    events::{
        status_changed::{build_status_changed_envelope, StatusChangedPayload},
        EVENT_TYPE_STATUS_CHANGED,
    },
};

/// Input for POST /api/inventory/status-transfers
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct StatusTransferRequest {
    pub tenant_id: String,
    pub item_id: Uuid,
    pub warehouse_id: Uuid,
    /// Source bucket
    pub from_status: InvItemStatus,
    /// Destination bucket (must differ from from_status)
    pub to_status: InvItemStatus,
    /// Quantity to transfer (must be > 0)
    pub quantity: i64,
    /// Caller-supplied idempotency key (scoped per tenant)
    pub idempotency_key: String,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
}

/// Result returned on successful or replayed transfer
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StatusTransferResult {
    /// Stable business key for this transfer (inv_status_transfers row id)
    pub transfer_id: Uuid,
    /// Outbox event id
    pub event_id: Uuid,
    pub tenant_id: String,
    pub item_id: Uuid,
    pub warehouse_id: Uuid,
    pub from_status: String,
    pub to_status: String,
    pub quantity: i64,
    pub transferred_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Error)]
pub enum StatusTransferError {
    #[error("Guard failed: {0}")]
    Guard(#[from] GuardError),

    #[error("from_status and to_status must differ")]
    SameStatus,

    #[error("Insufficient stock in {status} bucket: have {available}, need {requested}")]
    InsufficientStock {
        status: String,
        available: i64,
        requested: i64,
    },

    #[error("No {0} bucket row found for this item/warehouse; cannot transfer")]
    BucketNotFound(String),

    #[error("Idempotency key conflict: same key used with a different request body")]
    ConflictingIdempotencyKey,

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

/// Move quantity between status buckets atomically.
///
/// Returns `(StatusTransferResult, is_replay)`.
/// - `is_replay = false`: new transfer created; HTTP 201.
/// - `is_replay = true`:  idempotency key matched; HTTP 200 with stored result.
pub async fn process_status_transfer(
    pool: &PgPool,
    req: &StatusTransferRequest,
) -> Result<(StatusTransferResult, bool), StatusTransferError> {
    // --- Stateless input validation ---
    validate_request(req)?;

    // --- Compute request hash for idempotency conflict detection ---
    let request_hash = serde_json::to_string(req)?;

    // --- Idempotency check (fast path for replays) ---
    if let Some(record) =
        repo::find_idempotency_key(pool, &req.tenant_id, &req.idempotency_key).await?
    {
        if record.request_hash != request_hash {
            return Err(StatusTransferError::ConflictingIdempotencyKey);
        }
        let result: StatusTransferResult = serde_json::from_str(&record.response_body)?;
        return Ok((result, true));
    }

    // --- DB guard: item must exist and be active ---
    let item = guard_item_active(pool, req.item_id, &req.tenant_id).await?;

    let transferred_at = Utc::now();
    let event_id = Uuid::new_v4();
    let correlation_id = req
        .correlation_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let mut tx = pool.begin().await?;

    // --- Guard + Mutation: decrement from_status bucket ---
    let from_str = req.from_status.as_str();
    let to_str = req.to_status.as_str();

    if req.from_status == InvItemStatus::Available {
        // For 'available', guard against reserved stock.
        let row =
            repo::lock_available_bucket(&mut tx, &req.tenant_id, req.item_id, req.warehouse_id)
                .await?;

        let avail = row.map(|r| r.quantity_available).unwrap_or(0);
        if avail < req.quantity {
            return Err(StatusTransferError::InsufficientStock {
                status: "available".to_string(),
                available: avail,
                requested: req.quantity,
            });
        }

        let rows_affected = repo::decrement_available_bucket(
            &mut tx,
            &req.tenant_id,
            req.item_id,
            req.warehouse_id,
            req.quantity,
        )
        .await?;

        if rows_affected == 0 {
            return Err(StatusTransferError::BucketNotFound("available".to_string()));
        }

        repo::decrement_item_on_hand_available(
            &mut tx,
            &req.tenant_id,
            req.item_id,
            req.warehouse_id,
            req.quantity,
        )
        .await?;
    } else {
        // Non-available bucket: check quantity_on_hand in that bucket.
        let row = repo::lock_non_available_bucket(
            &mut tx,
            &req.tenant_id,
            req.item_id,
            req.warehouse_id,
            from_str,
        )
        .await?;

        let on_hand = match row {
            Some(r) => r.quantity_on_hand,
            None => return Err(StatusTransferError::BucketNotFound(from_str.to_string())),
        };

        if on_hand < req.quantity {
            return Err(StatusTransferError::InsufficientStock {
                status: from_str.to_string(),
                available: on_hand,
                requested: req.quantity,
            });
        }

        repo::decrement_non_available_bucket(
            &mut tx,
            &req.tenant_id,
            req.item_id,
            req.warehouse_id,
            req.quantity,
            from_str,
        )
        .await?;
    }

    // --- Increment to_status bucket (upsert) ---
    repo::upsert_to_bucket(
        &mut tx,
        &req.tenant_id,
        req.item_id,
        req.warehouse_id,
        to_str,
        req.quantity,
    )
    .await?;

    // If to_status == 'available', sync item_on_hand.available_status_on_hand
    if req.to_status == InvItemStatus::Available {
        repo::increment_item_on_hand_available(
            &mut tx,
            &req.tenant_id,
            req.item_id,
            req.warehouse_id,
            req.quantity,
        )
        .await?;
    }

    // --- Insert append-only ledger row ---
    let transfer_id = repo::insert_status_transfer(
        &mut tx,
        &req.tenant_id,
        req.item_id,
        req.warehouse_id,
        from_str,
        to_str,
        req.quantity,
        event_id,
        transferred_at,
    )
    .await?;

    // --- Build event envelope and enqueue in outbox ---
    let payload = StatusChangedPayload {
        transfer_id,
        tenant_id: req.tenant_id.clone(),
        item_id: req.item_id,
        sku: item.sku,
        warehouse_id: req.warehouse_id,
        from_status: from_str.to_string(),
        to_status: to_str.to_string(),
        quantity: req.quantity,
        transferred_at,
    };

    let envelope = build_status_changed_envelope(
        event_id,
        req.tenant_id.clone(),
        correlation_id.clone(),
        req.causation_id.clone(),
        payload,
    );
    let envelope_json = serde_json::to_string(&envelope)?;

    repo::insert_outbox_event(
        &mut tx,
        event_id,
        EVENT_TYPE_STATUS_CHANGED,
        &req.item_id.to_string(),
        &req.tenant_id,
        &envelope_json,
        &correlation_id,
        req.causation_id.as_deref(),
    )
    .await?;

    // --- Build result ---
    let result = StatusTransferResult {
        transfer_id,
        event_id,
        tenant_id: req.tenant_id.clone(),
        item_id: req.item_id,
        warehouse_id: req.warehouse_id,
        from_status: from_str.to_string(),
        to_status: to_str.to_string(),
        quantity: req.quantity,
        transferred_at,
    };

    // --- Store idempotency key (expires in 7 days) ---
    let response_json = serde_json::to_string(&result)?;
    let expires_at = transferred_at + Duration::days(7);

    repo::store_idempotency_key(
        &mut tx,
        &req.tenant_id,
        &req.idempotency_key,
        &request_hash,
        &response_json,
        expires_at,
    )
    .await?;

    tx.commit().await?;

    Ok((result, false))
}

fn validate_request(req: &StatusTransferRequest) -> Result<(), StatusTransferError> {
    if req.idempotency_key.trim().is_empty() {
        return Err(StatusTransferError::Guard(GuardError::Validation(
            "idempotency_key is required".to_string(),
        )));
    }
    if req.tenant_id.trim().is_empty() {
        return Err(StatusTransferError::Guard(GuardError::Validation(
            "tenant_id is required".to_string(),
        )));
    }
    if req.from_status == req.to_status {
        return Err(StatusTransferError::SameStatus);
    }
    guard_quantity_positive(req.quantity)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_req() -> StatusTransferRequest {
        StatusTransferRequest {
            tenant_id: "tenant-1".to_string(),
            item_id: Uuid::new_v4(),
            warehouse_id: Uuid::new_v4(),
            from_status: InvItemStatus::Available,
            to_status: InvItemStatus::Quarantine,
            quantity: 10,
            idempotency_key: "idem-001".to_string(),
            correlation_id: None,
            causation_id: None,
        }
    }

    #[test]
    fn validate_rejects_same_status() {
        let mut r = valid_req();
        r.from_status = InvItemStatus::Quarantine;
        r.to_status = InvItemStatus::Quarantine;
        assert!(matches!(
            validate_request(&r),
            Err(StatusTransferError::SameStatus)
        ));
    }

    #[test]
    fn validate_rejects_empty_idempotency_key() {
        let mut r = valid_req();
        r.idempotency_key = "  ".to_string();
        assert!(matches!(
            validate_request(&r),
            Err(StatusTransferError::Guard(_))
        ));
    }

    #[test]
    fn validate_rejects_empty_tenant() {
        let mut r = valid_req();
        r.tenant_id = "".to_string();
        assert!(matches!(
            validate_request(&r),
            Err(StatusTransferError::Guard(_))
        ));
    }

    #[test]
    fn validate_rejects_zero_quantity() {
        let mut r = valid_req();
        r.quantity = 0;
        assert!(matches!(
            validate_request(&r),
            Err(StatusTransferError::Guard(_))
        ));
    }

    #[test]
    fn validate_accepts_valid_request() {
        assert!(validate_request(&valid_req()).is_ok());
    }
}
