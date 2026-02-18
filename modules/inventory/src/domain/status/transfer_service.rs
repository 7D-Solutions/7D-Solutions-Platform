//! Status bucket transfer service (Guard → Mutation → Outbox atomicity).
//!
//! Moves quantity between status buckets (available | quarantine | damaged).
//! Available transfers guard against reserved stock (uses quantity_available).
//! Idempotency via `inv_idempotency_keys`. Append-only ledger in `inv_status_transfers`.

use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

use crate::{
    domain::guards::{GuardError, guard_item_active, guard_quantity_positive},
    domain::status::models::InvItemStatus,
    events::{
        EVENT_TYPE_STATUS_CHANGED,
        status_changed::{StatusChangedPayload, build_status_changed_envelope},
    },
};

/// Input for POST /api/inventory/status-transfers
#[derive(Debug, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(sqlx::FromRow)]
struct IdempotencyRecord {
    response_body: String,
    request_hash: String,
}

#[derive(sqlx::FromRow)]
struct AvailRow {
    quantity_available: i64,
}

#[derive(sqlx::FromRow)]
struct BucketRow {
    quantity_on_hand: i64,
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
    if let Some(record) = find_idempotency_key(pool, &req.tenant_id, &req.idempotency_key).await? {
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
        // Lock item_on_hand and check quantity_available (non-reserved available).
        let row = sqlx::query_as::<_, AvailRow>(
            r#"
            SELECT quantity_available
            FROM item_on_hand
            WHERE tenant_id = $1 AND item_id = $2 AND warehouse_id = $3
              AND location_id IS NULL
            FOR UPDATE
            "#,
        )
        .bind(&req.tenant_id)
        .bind(req.item_id)
        .bind(req.warehouse_id)
        .fetch_optional(&mut *tx)
        .await?;

        let avail = row.map(|r| r.quantity_available).unwrap_or(0);
        if avail < req.quantity {
            return Err(StatusTransferError::InsufficientStock {
                status: "available".to_string(),
                available: avail,
                requested: req.quantity,
            });
        }

        // Decrement available bucket in item_on_hand_by_status
        let rows = sqlx::query(
            r#"
            UPDATE item_on_hand_by_status
            SET quantity_on_hand = quantity_on_hand - $4,
                updated_at       = NOW()
            WHERE tenant_id    = $1
              AND item_id      = $2
              AND warehouse_id = $3
              AND status       = 'available'
              AND quantity_on_hand >= $4
            "#,
        )
        .bind(&req.tenant_id)
        .bind(req.item_id)
        .bind(req.warehouse_id)
        .bind(req.quantity)
        .execute(&mut *tx)
        .await?;

        if rows.rows_affected() == 0 {
            return Err(StatusTransferError::BucketNotFound("available".to_string()));
        }

        // Sync item_on_hand.available_status_on_hand
        sqlx::query(
            r#"
            UPDATE item_on_hand
            SET available_status_on_hand = available_status_on_hand - $4,
                projected_at             = NOW()
            WHERE tenant_id    = $1
              AND item_id      = $2
              AND warehouse_id = $3
              AND location_id IS NULL
            "#,
        )
        .bind(&req.tenant_id)
        .bind(req.item_id)
        .bind(req.warehouse_id)
        .bind(req.quantity)
        .execute(&mut *tx)
        .await?;
    } else {
        // Non-available bucket: check quantity_on_hand in that bucket.
        let row = sqlx::query_as::<_, BucketRow>(
            r#"
            SELECT quantity_on_hand
            FROM item_on_hand_by_status
            WHERE tenant_id    = $1
              AND item_id      = $2
              AND warehouse_id = $3
              AND status       = $4::inv_item_status
            FOR UPDATE
            "#,
        )
        .bind(&req.tenant_id)
        .bind(req.item_id)
        .bind(req.warehouse_id)
        .bind(from_str)
        .fetch_optional(&mut *tx)
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

        sqlx::query(
            r#"
            UPDATE item_on_hand_by_status
            SET quantity_on_hand = quantity_on_hand - $4,
                updated_at       = NOW()
            WHERE tenant_id    = $1
              AND item_id      = $2
              AND warehouse_id = $3
              AND status       = $5::inv_item_status
            "#,
        )
        .bind(&req.tenant_id)
        .bind(req.item_id)
        .bind(req.warehouse_id)
        .bind(req.quantity)
        .bind(from_str)
        .execute(&mut *tx)
        .await?;
    }

    // --- Increment to_status bucket (upsert) ---
    sqlx::query(
        r#"
        INSERT INTO item_on_hand_by_status
            (tenant_id, item_id, warehouse_id, status, quantity_on_hand)
        VALUES ($1, $2, $3, $4::inv_item_status, $5)
        ON CONFLICT (tenant_id, item_id, warehouse_id, status) DO UPDATE
            SET quantity_on_hand = item_on_hand_by_status.quantity_on_hand + $5,
                updated_at       = NOW()
        "#,
    )
    .bind(&req.tenant_id)
    .bind(req.item_id)
    .bind(req.warehouse_id)
    .bind(to_str)
    .bind(req.quantity)
    .execute(&mut *tx)
    .await?;

    // If to_status == 'available', sync item_on_hand.available_status_on_hand
    if req.to_status == InvItemStatus::Available {
        sqlx::query(
            r#"
            UPDATE item_on_hand
            SET available_status_on_hand = available_status_on_hand + $4,
                projected_at             = NOW()
            WHERE tenant_id    = $1
              AND item_id      = $2
              AND warehouse_id = $3
              AND location_id IS NULL
            "#,
        )
        .bind(&req.tenant_id)
        .bind(req.item_id)
        .bind(req.warehouse_id)
        .bind(req.quantity)
        .execute(&mut *tx)
        .await?;
    }

    // --- Insert append-only ledger row ---
    let transfer_id = sqlx::query_scalar::<_, Uuid>(
        r#"
        INSERT INTO inv_status_transfers
            (tenant_id, item_id, warehouse_id, from_status, to_status, quantity, event_id, transferred_at)
        VALUES
            ($1, $2, $3, $4::inv_item_status, $5::inv_item_status, $6, $7, $8)
        RETURNING id
        "#,
    )
    .bind(&req.tenant_id)
    .bind(req.item_id)
    .bind(req.warehouse_id)
    .bind(from_str)
    .bind(to_str)
    .bind(req.quantity)
    .bind(event_id)
    .bind(transferred_at)
    .fetch_one(&mut *tx)
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
    .bind(EVENT_TYPE_STATUS_CHANGED)
    .bind(req.item_id.to_string())
    .bind(&req.tenant_id)
    .bind(&envelope_json)
    .bind(&correlation_id)
    .bind(&req.causation_id)
    .execute(&mut *tx)
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
        assert!(matches!(validate_request(&r), Err(StatusTransferError::SameStatus)));
    }

    #[test]
    fn validate_rejects_empty_idempotency_key() {
        let mut r = valid_req();
        r.idempotency_key = "  ".to_string();
        assert!(matches!(validate_request(&r), Err(StatusTransferError::Guard(_))));
    }

    #[test]
    fn validate_rejects_empty_tenant() {
        let mut r = valid_req();
        r.tenant_id = "".to_string();
        assert!(matches!(validate_request(&r), Err(StatusTransferError::Guard(_))));
    }

    #[test]
    fn validate_rejects_zero_quantity() {
        let mut r = valid_req();
        r.quantity = 0;
        assert!(matches!(validate_request(&r), Err(StatusTransferError::Guard(_))));
    }

    #[test]
    fn validate_accepts_valid_request() {
        assert!(validate_request(&valid_req()).is_ok());
    }
}
