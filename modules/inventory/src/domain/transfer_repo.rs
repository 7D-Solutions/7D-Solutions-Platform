//! Transfer repository — database operations for inter-warehouse stock transfers.
//!
//! All transactional functions accept `&mut Transaction<'_, Postgres>` to
//! participate in the Guard → Lock → FIFO → Mutation → Outbox atomic unit
//! orchestrated by `transfer_service`.

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
pub(crate) struct LedgerRow {
    pub id: i64,
}

#[derive(sqlx::FromRow)]
pub(crate) struct LayerRow {
    pub id: Uuid,
    pub quantity_remaining: i64,
    pub unit_cost_minor: i64,
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
        VALUES ($1, $2, $3, $4::JSONB, 201, $5)
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
// FIFO layer reads
// ============================================================================

/// Lock all FIFO layers for the source warehouse item in FIFO order.
pub(crate) async fn lock_fifo_layers(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
) -> Result<Vec<LayerRow>, sqlx::Error> {
    sqlx::query_as::<_, LayerRow>(
        r#"
        SELECT id, quantity_remaining, unit_cost_minor
        FROM inventory_layers
        WHERE tenant_id     = $1
          AND item_id       = $2
          AND warehouse_id  = $3
          AND quantity_remaining > 0
        ORDER BY received_at ASC, ledger_entry_id ASC
        FOR UPDATE
        "#,
    )
    .bind(tenant_id)
    .bind(item_id)
    .bind(warehouse_id)
    .fetch_all(&mut **tx)
    .await
}

/// Fetch reserved quantity for the warehouse-level on-hand row (no location).
pub(crate) async fn fetch_quantity_reserved(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        r#"
        SELECT COALESCE(quantity_reserved, 0)
        FROM item_on_hand
        WHERE tenant_id    = $1
          AND item_id      = $2
          AND warehouse_id = $3
          AND location_id IS NULL
        "#,
    )
    .bind(tenant_id)
    .bind(item_id)
    .bind(warehouse_id)
    .fetch_optional(&mut **tx)
    .await
    .map(|opt| opt.unwrap_or(0i64))
}

// ============================================================================
// Ledger inserts
// ============================================================================

/// Insert a 'transfer_out' ledger row (source debit). Returns ledger id.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn insert_transfer_out_ledger(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
    quantity: i64,
    currency: &str,
    out_event_id: Uuid,
    event_type: &str,
    transfer_id: &str,
    transferred_at: DateTime<Utc>,
) -> Result<i64, sqlx::Error> {
    let row = sqlx::query_as::<_, LedgerRow>(
        r#"
        INSERT INTO inventory_ledger
            (tenant_id, item_id, warehouse_id, location_id, entry_type, quantity,
             unit_cost_minor, currency, source_event_id, source_event_type,
             reference_type, reference_id, posted_at)
        VALUES
            ($1, $2, $3, NULL, 'transfer_out', $4, 0, $5, $6, $7, 'transfer', $8, $9)
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(item_id)
    .bind(warehouse_id)
    .bind(quantity)
    .bind(currency)
    .bind(out_event_id)
    .bind(event_type)
    .bind(transfer_id)
    .bind(transferred_at)
    .fetch_one(&mut **tx)
    .await?;
    Ok(row.id)
}

/// Insert a 'transfer_in' ledger row (destination credit). Returns ledger id.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn insert_transfer_in_ledger(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
    quantity: i64,
    avg_unit_cost: i64,
    currency: &str,
    in_event_id: Uuid,
    event_type: &str,
    transfer_id: &str,
    transferred_at: DateTime<Utc>,
) -> Result<i64, sqlx::Error> {
    let row = sqlx::query_as::<_, LedgerRow>(
        r#"
        INSERT INTO inventory_ledger
            (tenant_id, item_id, warehouse_id, location_id, entry_type, quantity,
             unit_cost_minor, currency, source_event_id, source_event_type,
             reference_type, reference_id, posted_at)
        VALUES
            ($1, $2, $3, NULL, 'transfer_in', $4, $5, $6, $7, $8, 'transfer', $9, $10)
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(item_id)
    .bind(warehouse_id)
    .bind(quantity)
    .bind(avg_unit_cost)
    .bind(currency)
    .bind(in_event_id)
    .bind(event_type)
    .bind(transfer_id)
    .bind(transferred_at)
    .fetch_one(&mut **tx)
    .await?;
    Ok(row.id)
}

// ============================================================================
// FIFO layer mutations
// ============================================================================

pub(crate) async fn insert_layer_consumption(
    tx: &mut Transaction<'_, Postgres>,
    layer_id: Uuid,
    ledger_entry_id: i64,
    quantity: i64,
    unit_cost_minor: i64,
    consumed_at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO layer_consumptions
            (layer_id, ledger_entry_id, quantity_consumed, unit_cost_minor, consumed_at)
        VALUES ($1, $2, $3, $4, $5)
        "#,
    )
    .bind(layer_id)
    .bind(ledger_entry_id)
    .bind(quantity)
    .bind(unit_cost_minor)
    .bind(consumed_at)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub(crate) async fn decrement_layer(
    tx: &mut Transaction<'_, Postgres>,
    quantity: i64,
    consumed_at: DateTime<Utc>,
    layer_id: Uuid,
) -> Result<(), sqlx::Error> {
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
    .bind(quantity)
    .bind(consumed_at)
    .bind(layer_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Insert a new FIFO layer at the destination warehouse.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn insert_destination_fifo_layer(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
    ledger_entry_id: i64,
    received_at: DateTime<Utc>,
    quantity: i64,
    avg_unit_cost: i64,
    currency: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO inventory_layers
            (tenant_id, item_id, warehouse_id, ledger_entry_id, received_at,
             quantity_received, quantity_remaining, unit_cost_minor, currency)
        VALUES
            ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        "#,
    )
    .bind(tenant_id)
    .bind(item_id)
    .bind(warehouse_id)
    .bind(ledger_entry_id)
    .bind(received_at)
    .bind(quantity)
    .bind(quantity)
    .bind(avg_unit_cost)
    .bind(currency)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

// ============================================================================
// Business record
// ============================================================================

#[allow(clippy::too_many_arguments)]
pub(crate) async fn insert_transfer_record(
    tx: &mut Transaction<'_, Postgres>,
    transfer_id: Uuid,
    tenant_id: &str,
    item_id: Uuid,
    from_warehouse_id: Uuid,
    to_warehouse_id: Uuid,
    quantity: i64,
    event_id: Uuid,
    issue_ledger_id: i64,
    receipt_ledger_id: i64,
    transferred_at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO inv_transfers
            (id, tenant_id, item_id, from_warehouse_id, to_warehouse_id,
             quantity, event_id, issue_ledger_id, receipt_ledger_id, transferred_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        "#,
    )
    .bind(transfer_id)
    .bind(tenant_id)
    .bind(item_id)
    .bind(from_warehouse_id)
    .bind(to_warehouse_id)
    .bind(quantity)
    .bind(event_id)
    .bind(issue_ledger_id)
    .bind(receipt_ledger_id)
    .bind(transferred_at)
    .execute(&mut **tx)
    .await?;
    Ok(())
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
