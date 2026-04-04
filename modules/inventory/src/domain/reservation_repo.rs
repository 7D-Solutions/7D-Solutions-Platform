//! Reservation repository — database operations for the reservation lifecycle.
//!
//! All transactional functions accept `&mut Transaction<'_, Postgres>` so they
//! participate in the Guard → Mutation → Outbox atomic unit established by
//! `reservation_service`.

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

/// Minimal reservation row fetched for the release guard check.
#[derive(sqlx::FromRow)]
pub(crate) struct ReservationRow {
    pub id: Uuid,
    pub tenant_id: String,
    pub item_id: Uuid,
    pub warehouse_id: Uuid,
    pub quantity: i64,
}

// ============================================================================
// Idempotency
// ============================================================================

/// Look up an existing idempotency key (fast-path replay check, outside tx).
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

/// Store an idempotency key with its response body (inside tx).
pub(crate) async fn store_idempotency_key(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    idempotency_key: &str,
    request_hash: &str,
    response_json: &str,
    status_code: i16,
    expires_at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO inv_idempotency_keys
            (tenant_id, idempotency_key, request_hash, response_body, status_code, expires_at)
        VALUES ($1, $2, $3, $4::JSONB, $5, $6)
        "#,
    )
    .bind(tenant_id)
    .bind(idempotency_key)
    .bind(request_hash)
    .bind(response_json)
    .bind(status_code)
    .bind(expires_at)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

// ============================================================================
// Available stock
// ============================================================================

/// Lock the on-hand row and return available stock. Returns `None` if no row exists.
/// Uses `SELECT FOR UPDATE` to serialize concurrent reservations.
pub(crate) async fn lock_available_stock(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar(
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
    .map(|opt| opt.flatten())
}

// ============================================================================
// Reservation CRUD
// ============================================================================

/// Insert a primary reservation row (status='active'). Returns the reservation id.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn insert_reservation(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
    quantity: i64,
    reference_type: Option<&str>,
    reference_id: Option<&str>,
    reserved_at: DateTime<Utc>,
    expires_at: Option<DateTime<Utc>>,
) -> Result<Uuid, sqlx::Error> {
    sqlx::query_scalar::<_, Uuid>(
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
    .bind(tenant_id)
    .bind(item_id)
    .bind(warehouse_id)
    .bind(quantity)
    .bind(reference_type)
    .bind(reference_id)
    .bind(reserved_at)
    .bind(expires_at)
    .fetch_one(&mut **tx)
    .await
}

/// Fetch the original reservation row for a release operation (must be primary entry).
pub(crate) async fn find_active_reservation(
    pool: &PgPool,
    reservation_id: Uuid,
    tenant_id: &str,
) -> Result<Option<ReservationRow>, sqlx::Error> {
    sqlx::query_as::<_, ReservationRow>(
        r#"
        SELECT id, tenant_id, item_id, warehouse_id, quantity
        FROM inventory_reservations
        WHERE id = $1 AND tenant_id = $2 AND reverses_reservation_id IS NULL
        "#,
    )
    .bind(reservation_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await
}

/// Check whether a compensating entry already exists for the given reservation.
pub(crate) async fn is_already_compensated(
    pool: &PgPool,
    reservation_id: Uuid,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM inventory_reservations WHERE reverses_reservation_id = $1)",
    )
    .bind(reservation_id)
    .fetch_one(pool)
    .await
}

/// Insert a compensating reservation row (status='released'). Returns the release id.
pub(crate) async fn insert_compensating_reservation(
    tx: &mut Transaction<'_, Postgres>,
    original: &ReservationRow,
    released_at: DateTime<Utc>,
) -> Result<Uuid, sqlx::Error> {
    sqlx::query_scalar::<_, Uuid>(
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
    .fetch_one(&mut **tx)
    .await
}

// ============================================================================
// On-hand projection
// ============================================================================

/// Upsert on-hand projection — increment quantity_reserved (inside tx).
pub(crate) async fn increment_reserved(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
    quantity: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO item_on_hand
            (tenant_id, item_id, warehouse_id, location_id, quantity_reserved, projected_at)
        VALUES ($1, $2, $3, NULL, $4, NOW())
        ON CONFLICT (tenant_id, item_id, warehouse_id)
        WHERE location_id IS NULL
        DO UPDATE
            SET quantity_reserved = item_on_hand.quantity_reserved + EXCLUDED.quantity_reserved,
                projected_at = NOW()
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

/// Decrement quantity_reserved on the null-location on-hand row (inside tx).
pub(crate) async fn decrement_reserved(
    tx: &mut Transaction<'_, Postgres>,
    original: &ReservationRow,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE item_on_hand
        SET quantity_reserved = GREATEST(0, quantity_reserved - $1),
            projected_at = NOW()
        WHERE tenant_id = $2 AND item_id = $3 AND warehouse_id = $4
          AND location_id IS NULL
        "#,
    )
    .bind(original.quantity)
    .bind(&original.tenant_id)
    .bind(original.item_id)
    .bind(original.warehouse_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

// ============================================================================
// Outbox
// ============================================================================

/// Insert an outbox event for a reservation operation (inside tx).
pub(crate) async fn insert_outbox_event(
    tx: &mut Transaction<'_, Postgres>,
    event_id: Uuid,
    event_type: &str,
    aggregate_id: &str,
    tenant_id: &str,
    payload_json: &str,
    correlation_id: &str,
    causation_id: Option<&str>,
) -> Result<(), sqlx::Error> {
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
    .bind(event_type)
    .bind(aggregate_id)
    .bind(tenant_id)
    .bind(payload_json)
    .bind(correlation_id)
    .bind(causation_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
