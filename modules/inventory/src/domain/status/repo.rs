//! Status repository — database operations for status bucket transfers.
//!
//! All transactional functions accept `&mut Transaction<'_, Postgres>` to
//! participate in the Guard → Mutation → Outbox atomic unit orchestrated by
//! `transfer_service`.

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

// ============================================================================
// Internal DB row types
// ============================================================================

#[derive(sqlx::FromRow)]
pub(crate) struct IdempotencyRecord {
    pub response_body: String,
    pub request_hash: String,
}

#[derive(sqlx::FromRow)]
pub(crate) struct AvailRow {
    pub quantity_available: i64,
}

#[derive(sqlx::FromRow)]
pub(crate) struct BucketRow {
    pub quantity_on_hand: i64,
}

// ============================================================================
// Idempotency
// ============================================================================

pub(crate) async fn find_idempotency_key(
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

pub(crate) async fn store_idempotency_key(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    idempotency_key: &str,
    request_hash: &str,
    response_json: &str,
    expires_at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO inv_idempotency_keys
            (tenant_id, idempotency_key, request_hash, response_body, status_code, expires_at)
        VALUES
            ($1, $2, $3, $4::JSONB, 201, $5)
        "#,
    )
    .bind(tenant_id)
    .bind(idempotency_key)
    .bind(request_hash)
    .bind(response_json)
    .bind(expires_at)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

// ============================================================================
// Available bucket guards / mutations
// ============================================================================

/// Lock the item_on_hand row and return quantity_available for the available bucket.
pub(crate) async fn lock_available_bucket(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
) -> Result<Option<AvailRow>, sqlx::Error> {
    sqlx::query_as::<_, AvailRow>(
        r#"
        SELECT quantity_available
        FROM item_on_hand
        WHERE tenant_id = $1 AND item_id = $2 AND warehouse_id = $3
          AND location_id IS NULL
        FOR UPDATE
        "#,
    )
    .bind(tenant_id)
    .bind(item_id)
    .bind(warehouse_id)
    .fetch_optional(&mut **tx)
    .await
}

/// Decrement the 'available' status bucket. Returns rows_affected (0 = not found or insufficient).
pub(crate) async fn decrement_available_bucket(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
    quantity: i64,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
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
    .bind(tenant_id)
    .bind(item_id)
    .bind(warehouse_id)
    .bind(quantity)
    .execute(&mut **tx)
    .await?;
    Ok(result.rows_affected())
}

/// Sync item_on_hand.available_status_on_hand after decrementing the available bucket.
pub(crate) async fn decrement_item_on_hand_available(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
    quantity: i64,
) -> Result<(), sqlx::Error> {
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
    .bind(tenant_id)
    .bind(item_id)
    .bind(warehouse_id)
    .bind(quantity)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Lock a non-available status bucket row and return its quantity.
pub(crate) async fn lock_non_available_bucket(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
    status: &str,
) -> Result<Option<BucketRow>, sqlx::Error> {
    sqlx::query_as::<_, BucketRow>(
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
    .bind(tenant_id)
    .bind(item_id)
    .bind(warehouse_id)
    .bind(status)
    .fetch_optional(&mut **tx)
    .await
}

pub(crate) async fn decrement_non_available_bucket(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
    quantity: i64,
    status: &str,
) -> Result<(), sqlx::Error> {
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
    .bind(tenant_id)
    .bind(item_id)
    .bind(warehouse_id)
    .bind(quantity)
    .bind(status)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Upsert (increment) the destination status bucket.
pub(crate) async fn upsert_to_bucket(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
    status: &str,
    quantity: i64,
) -> Result<(), sqlx::Error> {
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
    .bind(tenant_id)
    .bind(item_id)
    .bind(warehouse_id)
    .bind(status)
    .bind(quantity)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Sync item_on_hand.available_status_on_hand after incrementing the available bucket.
pub(crate) async fn increment_item_on_hand_available(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
    quantity: i64,
) -> Result<(), sqlx::Error> {
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
    .bind(tenant_id)
    .bind(item_id)
    .bind(warehouse_id)
    .bind(quantity)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

// ============================================================================
// Business record
// ============================================================================

/// Insert an append-only status transfer ledger row. Returns the transfer id.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn insert_status_transfer(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
    from_status: &str,
    to_status: &str,
    quantity: i64,
    event_id: Uuid,
    transferred_at: DateTime<Utc>,
) -> Result<Uuid, sqlx::Error> {
    sqlx::query_scalar::<_, Uuid>(
        r#"
        INSERT INTO inv_status_transfers
            (tenant_id, item_id, warehouse_id, from_status, to_status, quantity, event_id, transferred_at)
        VALUES
            ($1, $2, $3, $4::inv_item_status, $5::inv_item_status, $6, $7, $8)
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(item_id)
    .bind(warehouse_id)
    .bind(from_status)
    .bind(to_status)
    .bind(quantity)
    .bind(event_id)
    .bind(transferred_at)
    .fetch_one(&mut **tx)
    .await
}

// ============================================================================
// Outbox
// ============================================================================

pub(crate) async fn insert_outbox_event(
    tx: &mut Transaction<'_, Postgres>,
    event_id: Uuid,
    event_type: &str,
    aggregate_id: &str,
    tenant_id: &str,
    envelope_json: &str,
    correlation_id: &str,
    causation_id: Option<&str>,
) -> Result<(), sqlx::Error> {
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
    .bind(event_type)
    .bind(aggregate_id)
    .bind(tenant_id)
    .bind(envelope_json)
    .bind(correlation_id)
    .bind(causation_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
