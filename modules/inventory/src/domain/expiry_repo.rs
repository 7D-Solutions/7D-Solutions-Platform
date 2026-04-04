//! Expiry repository — database operations for lot expiry assignment and alert scanning.
//!
//! `LotExpiryRecord` is defined here (it has `sqlx::FromRow`) and re-exported
//! from `expiry.rs` for API compatibility. All `sqlx::query` calls live here.

use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};
use utoipa::ToSchema;
use uuid::Uuid;

// ============================================================================
// Domain model (re-exported via expiry.rs)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct LotExpiryRecord {
    pub lot_id: Uuid,
    pub tenant_id: String,
    pub item_id: Uuid,
    pub lot_code: String,
    pub expires_on: NaiveDate,
    pub expiry_source: String,
    pub expiry_set_at: DateTime<Utc>,
}

// ============================================================================
// Internal DB row types
// ============================================================================

#[derive(sqlx::FromRow)]
pub(crate) struct IdempotencyRecord {
    pub response_body: String,
    pub request_hash: String,
}

#[derive(sqlx::FromRow)]
pub(crate) struct LotRow {
    pub item_id: Uuid,
    pub created_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
pub(crate) struct LotCandidate {
    pub id: Uuid,
    pub item_id: Uuid,
    pub lot_code: String,
    pub expires_on: NaiveDate,
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

/// Store an idempotency key using a pool (no active transaction — for scan results).
pub(crate) async fn store_idempotency_key_pool(
    pool: &PgPool,
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
    .execute(pool)
    .await?;
    Ok(())
}

// ============================================================================
// Policy reads
// ============================================================================

/// Fetch shelf_life_days from the effective revision at a reference time.
pub(crate) async fn fetch_shelf_life_days(
    pool: &PgPool,
    tenant_id: &str,
    item_id: Uuid,
    reference_at: DateTime<Utc>,
) -> Result<Option<i32>, sqlx::Error> {
    sqlx::query_scalar(
        r#"
        SELECT shelf_life_days
        FROM item_revisions
        WHERE tenant_id = $1 AND item_id = $2
          AND effective_from IS NOT NULL
          AND effective_from <= $3
          AND (effective_to IS NULL OR effective_to > $3)
        ORDER BY effective_from DESC
        LIMIT 1
        "#,
    )
    .bind(tenant_id)
    .bind(item_id)
    .bind(reference_at)
    .fetch_optional(pool)
    .await
    .map(|opt| opt.flatten())
}

// ============================================================================
// Lot mutations
// ============================================================================

/// Lock the lot row for update. Returns None if not found.
pub(crate) async fn lock_lot(
    tx: &mut Transaction<'_, Postgres>,
    lot_id: Uuid,
    tenant_id: &str,
) -> Result<Option<LotRow>, sqlx::Error> {
    sqlx::query_as::<_, LotRow>(
        r#"
        SELECT item_id, created_at
        FROM inventory_lots
        WHERE id = $1 AND tenant_id = $2
        FOR UPDATE
        "#,
    )
    .bind(lot_id)
    .bind(tenant_id)
    .fetch_optional(&mut **tx)
    .await
}

/// Update the lot expiry fields and return the updated record.
pub(crate) async fn update_lot_expiry(
    tx: &mut Transaction<'_, Postgres>,
    expires_on: NaiveDate,
    expiry_source: &str,
    now: DateTime<Utc>,
    lot_id: Uuid,
    tenant_id: &str,
) -> Result<LotExpiryRecord, sqlx::Error> {
    sqlx::query_as::<_, LotExpiryRecord>(
        r#"
        UPDATE inventory_lots
        SET expires_on = $1,
            expiry_source = $2,
            expiry_set_at = $3
        WHERE id = $4 AND tenant_id = $5
        RETURNING
            id AS lot_id,
            tenant_id,
            item_id,
            lot_code,
            expires_on,
            COALESCE(expiry_source, '') AS expiry_source,
            COALESCE(expiry_set_at, NOW()) AS expiry_set_at
        "#,
    )
    .bind(expires_on)
    .bind(expiry_source)
    .bind(now)
    .bind(lot_id)
    .bind(tenant_id)
    .fetch_one(&mut **tx)
    .await
}

pub(crate) async fn insert_lot_outbox_event(
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
        VALUES ($1, $2, 'inventory_lot', $3, $4, $5::JSONB, $6, $7, '1.0.0')
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

// ============================================================================
// Alert scan reads
// ============================================================================

/// Fetch lots expiring soon (within the window, not yet expired).
pub(crate) async fn fetch_expiring_lots(
    pool: &PgPool,
    tenant_id: &str,
    as_of_date: NaiveDate,
    expiring_within_days: i32,
) -> Result<Vec<LotCandidate>, sqlx::Error> {
    sqlx::query_as::<_, LotCandidate>(
        r#"
        SELECT id, item_id, lot_code, expires_on
        FROM inventory_lots
        WHERE tenant_id = $1
          AND expires_on IS NOT NULL
          AND expires_on > $2
          AND expires_on <= $3
        "#,
    )
    .bind(tenant_id)
    .bind(as_of_date)
    .bind(as_of_date + Duration::days(expiring_within_days as i64))
    .fetch_all(pool)
    .await
}

/// Fetch lots already expired as of the given date.
pub(crate) async fn fetch_expired_lots(
    pool: &PgPool,
    tenant_id: &str,
    as_of_date: NaiveDate,
) -> Result<Vec<LotCandidate>, sqlx::Error> {
    sqlx::query_as::<_, LotCandidate>(
        r#"
        SELECT id, item_id, lot_code, expires_on
        FROM inventory_lots
        WHERE tenant_id = $1
          AND expires_on IS NOT NULL
          AND expires_on <= $2
        "#,
    )
    .bind(tenant_id)
    .bind(as_of_date)
    .fetch_all(pool)
    .await
}

// ============================================================================
// Alert state mutations
// ============================================================================

/// Insert alert state marker if not already present. Returns the new id if inserted.
pub(crate) async fn insert_alert_state_if_new(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    lot_id: Uuid,
    alert_type: &str,
    alert_date: NaiveDate,
    window_days: i32,
) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar(
        r#"
        INSERT INTO inv_lot_expiry_alert_state
            (tenant_id, lot_id, alert_type, alert_date, window_days)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (tenant_id, lot_id, alert_type, alert_date, window_days)
        DO NOTHING
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(lot_id)
    .bind(alert_type)
    .bind(alert_date)
    .bind(window_days)
    .fetch_optional(&mut **tx)
    .await
}

pub(crate) async fn insert_alert_outbox_event(
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
        VALUES ($1, $2, 'inventory_lot', $3, $4, $5::JSONB, $6, $7, '1.0.0')
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
