//! Receipt repository — database operations for the receipt transaction.
//!
//! All functions run within an existing `sqlx::Transaction` (or accept
//! `&PgPool` for non-transactional reads) so they participate in the
//! Guard → Mutation → Outbox atomic unit established by `receipt_service`.

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

// ============================================================================
// Internal DB row types
// ============================================================================

#[derive(sqlx::FromRow)]
pub(crate) struct LedgerRow {
    pub id: i64,
    pub entry_id: Uuid,
}

#[derive(sqlx::FromRow)]
pub(crate) struct IdempotencyRecord {
    pub response_body: String,
    pub request_hash: String,
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
// Ledger
// ============================================================================

/// Insert a ledger row for a receipt. Returns `(id, entry_id)`.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn insert_ledger_row(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
    location_id: Option<Uuid>,
    quantity: i64,
    unit_cost_minor: i64,
    currency: &str,
    event_id: Uuid,
    event_type: &str,
    purchase_order_id: Option<Uuid>,
    source_type: &str,
    posted_at: DateTime<Utc>,
) -> Result<LedgerRow, sqlx::Error> {
    sqlx::query_as::<_, LedgerRow>(
        r#"
        INSERT INTO inventory_ledger
            (tenant_id, item_id, warehouse_id, location_id, entry_type, quantity,
             unit_cost_minor, currency, source_event_id, source_event_type,
             reference_type, reference_id, source_type, posted_at)
        VALUES
            ($1, $2, $3, $4, 'received', $5, $6, $7, $8, $9, $10, $11, $12, $13)
        RETURNING id, entry_id
        "#,
    )
    .bind(tenant_id)
    .bind(item_id)
    .bind(warehouse_id)
    .bind(location_id)
    .bind(quantity)
    .bind(unit_cost_minor)
    .bind(currency)
    .bind(event_id)
    .bind(event_type)
    .bind(purchase_order_id.map(|_| "purchase_order"))
    .bind(purchase_order_id.map(|id| id.to_string()))
    .bind(source_type)
    .bind(posted_at)
    .fetch_one(&mut **tx)
    .await
}

// ============================================================================
// FIFO Layer
// ============================================================================

/// Insert a FIFO layer for a receipt. Returns the layer id.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn insert_fifo_layer(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
    ledger_entry_id: i64,
    received_at: DateTime<Utc>,
    quantity: i64,
    unit_cost_minor: i64,
    currency: &str,
    lot_id: Option<Uuid>,
) -> Result<Uuid, sqlx::Error> {
    sqlx::query_scalar::<_, Uuid>(
        r#"
        INSERT INTO inventory_layers
            (tenant_id, item_id, warehouse_id, ledger_entry_id, received_at,
             quantity_received, quantity_remaining, unit_cost_minor, currency, lot_id)
        VALUES
            ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(item_id)
    .bind(warehouse_id)
    .bind(ledger_entry_id)
    .bind(received_at)
    .bind(quantity)
    .bind(quantity) // quantity_remaining = quantity_received on insert
    .bind(unit_cost_minor)
    .bind(currency)
    .bind(lot_id)
    .fetch_one(&mut **tx)
    .await
}

// ============================================================================
// Outbox
// ============================================================================

/// Insert an outbox event for a receipt.
pub(crate) async fn insert_outbox_event(
    tx: &mut Transaction<'_, Postgres>,
    event_id: Uuid,
    event_type: &str,
    aggregate_type: &str,
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
            ($1, $2, $3, $4, $5, $6::JSONB, $7, $8, '1.0.0')
        "#,
    )
    .bind(event_id)
    .bind(event_type)
    .bind(aggregate_type)
    .bind(aggregate_id)
    .bind(tenant_id)
    .bind(envelope_json)
    .bind(correlation_id)
    .bind(causation_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
