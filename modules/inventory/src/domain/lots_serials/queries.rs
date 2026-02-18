//! Read-only queries for lot and serial instance traceability.
//!
//! All queries are tenant-scoped and rely on indexed columns for performance.
//!
//! ## Lot trace
//! A lot's movement history = the receipt ledger entry (via layer) +
//! all issue/consumption entries (via layer_consumptions for that lot's layers).
//!
//! ## Serial trace
//! A serial's movement history = the receipt ledger entry stored on the
//! serial instance + all issue entries for the layer the serial occupied.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use super::models::{InventoryLot, InventorySerialInstance};

// ============================================================================
// Types
// ============================================================================

/// A ledger movement row returned by trace queries.
///
/// Represents a single `inventory_ledger` entry linked to a lot or serial.
/// Callers receive movements in ledger-id (chronological) order.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct LedgerMovement {
    pub ledger_id: i64,
    /// e.g. "received", "issued", "adjusted", "transfer_in", "transfer_out"
    pub entry_type: String,
    /// Signed quantity: positive = stock in, negative = stock out.
    pub quantity: i64,
    pub unit_cost_minor: i64,
    pub currency: String,
    pub reference_type: Option<String>,
    pub reference_id: Option<String>,
    pub posted_at: DateTime<Utc>,
}

// ============================================================================
// Lot queries
// ============================================================================

/// Return all lots for the given tenant and item, ordered by creation time.
///
/// Uses `idx_lots_tenant_item` index for efficient tenant-scoped lookup.
/// Returns an empty vec when no lots exist (not an error).
pub async fn list_lots_for_item(
    pool: &PgPool,
    tenant_id: &str,
    item_id: Uuid,
) -> Result<Vec<InventoryLot>, sqlx::Error> {
    sqlx::query_as::<_, InventoryLot>(
        r#"
        SELECT id, tenant_id, item_id, lot_code, attributes, created_at
        FROM inventory_lots
        WHERE tenant_id = $1
          AND item_id   = $2
        ORDER BY created_at ASC, id ASC
        "#,
    )
    .bind(tenant_id)
    .bind(item_id)
    .fetch_all(pool)
    .await
}

/// Return all ledger movements linked to `lot_code` for (tenant, item).
///
/// Includes:
///   1. The `received` entry for each FIFO layer created under this lot.
///   2. All `issued` / other entries that consumed from those layers
///      (via `layer_consumptions`).
///
/// Returns an empty vec when the lot does not exist — callers may distinguish
/// "lot not found" from "lot has no movements" using `list_lots_for_item`.
///
/// Index coverage:
///   - `idx_lots_tenant_item`      — lot lookup
///   - `idx_layers_lot_id`         — layer → lot join
///   - `idx_consumptions_layer_id` — consumption → layer join
///   - `idx_ledger_tenant_item_seq`— final ledger fetch ordered by id
pub async fn trace_lot(
    pool: &PgPool,
    tenant_id: &str,
    item_id: Uuid,
    lot_code: &str,
) -> Result<Vec<LedgerMovement>, sqlx::Error> {
    sqlx::query_as::<_, LedgerMovement>(
        r#"
        SELECT l.id             AS ledger_id,
               l.entry_type::TEXT,
               l.quantity,
               l.unit_cost_minor,
               l.currency,
               l.reference_type,
               l.reference_id,
               l.posted_at
        FROM inventory_ledger l
        WHERE l.tenant_id = $1
          AND l.item_id   = $2
          AND l.id IN (
              -- Receipt entry that created each layer belonging to this lot
              SELECT il.ledger_entry_id
              FROM inventory_layers il
              INNER JOIN inventory_lots lot
                      ON lot.id = il.lot_id
              WHERE lot.tenant_id = $1
                AND lot.item_id   = $2
                AND lot.lot_code  = $3

              UNION ALL

              -- Issue / other consumption entries for layers of this lot
              SELECT lc.ledger_entry_id
              FROM layer_consumptions lc
              INNER JOIN inventory_layers il ON il.id = lc.layer_id
              INNER JOIN inventory_lots lot  ON lot.id = il.lot_id
              WHERE lot.tenant_id = $1
                AND lot.item_id   = $2
                AND lot.lot_code  = $3
          )
        ORDER BY l.id ASC
        "#,
    )
    .bind(tenant_id)
    .bind(item_id)
    .bind(lot_code)
    .fetch_all(pool)
    .await
}

// ============================================================================
// Serial queries
// ============================================================================

/// Return all serial instances for (tenant, item), ordered by creation time.
///
/// Uses `idx_serials_tenant_item` index. Returns an empty vec when none exist.
pub async fn list_serials_for_item(
    pool: &PgPool,
    tenant_id: &str,
    item_id: Uuid,
) -> Result<Vec<InventorySerialInstance>, sqlx::Error> {
    sqlx::query_as::<_, InventorySerialInstance>(
        r#"
        SELECT id, tenant_id, item_id, serial_code,
               receipt_ledger_entry_id, layer_id, status, created_at
        FROM inventory_serial_instances
        WHERE tenant_id = $1
          AND item_id   = $2
        ORDER BY created_at ASC, id ASC
        "#,
    )
    .bind(tenant_id)
    .bind(item_id)
    .fetch_all(pool)
    .await
}

/// Return all ledger movements linked to `serial_code` for (tenant, item).
///
/// Includes:
///   1. The `received` entry stored on the serial instance itself.
///   2. All entries that consumed from the layer the serial occupied
///      (via `layer_consumptions`).
///
/// Returns an empty vec when the serial code does not exist.
///
/// Index coverage:
///   - `idx_serials_tenant_item`      — serial lookup
///   - `idx_consumptions_layer_id`    — consumption → layer join
///   - `idx_ledger_tenant_item_seq`   — final ledger fetch
pub async fn trace_serial(
    pool: &PgPool,
    tenant_id: &str,
    item_id: Uuid,
    serial_code: &str,
) -> Result<Vec<LedgerMovement>, sqlx::Error> {
    sqlx::query_as::<_, LedgerMovement>(
        r#"
        SELECT l.id             AS ledger_id,
               l.entry_type::TEXT,
               l.quantity,
               l.unit_cost_minor,
               l.currency,
               l.reference_type,
               l.reference_id,
               l.posted_at
        FROM inventory_ledger l
        WHERE l.tenant_id = $1
          AND l.item_id   = $2
          AND l.id IN (
              -- Receipt entry that created this serial instance
              SELECT si.receipt_ledger_entry_id
              FROM inventory_serial_instances si
              WHERE si.tenant_id   = $1
                AND si.item_id     = $2
                AND si.serial_code = $3

              UNION ALL

              -- All consumption entries for the layer this serial occupied
              SELECT lc.ledger_entry_id
              FROM layer_consumptions lc
              INNER JOIN inventory_serial_instances si
                      ON si.layer_id = lc.layer_id
              WHERE si.tenant_id   = $1
                AND si.item_id     = $2
                AND si.serial_code = $3
          )
        ORDER BY l.id ASC
        "#,
    )
    .bind(tenant_id)
    .bind(item_id)
    .bind(serial_code)
    .fetch_all(pool)
    .await
}
