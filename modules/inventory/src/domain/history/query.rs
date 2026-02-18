//! Movement history query — read-only ledger audit.
//!
//! Returns all `inventory_ledger` entries for a (tenant, item), optionally
//! narrowed to a specific location.  Results are ordered deterministically:
//! `posted_at ASC`, `id ASC` as a tie-breaker.
//!
//! ## Index coverage
//! - `idx_ledger_tenant_item_seq` — covers (tenant_id, item_id, id) for
//!   the unfiltered path.
//! - `idx_ledger_location` — covers (tenant_id, item_id, location_id) WHERE
//!   location_id IS NOT NULL for the location-filtered path.
//!
//! No writes are performed; the function is safe to call concurrently.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

// ============================================================================
// Types
// ============================================================================

/// A single ledger movement returned by the history query.
///
/// Includes both the business reference (`reference_type` / `reference_id`)
/// and the originating event reference (`source_event_id` / `source_event_type`).
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct MovementEntry {
    /// Stable monotonic ordering key (BIGSERIAL primary key of inventory_ledger).
    pub ledger_id: i64,
    /// Immutable UUID for the ledger row (unique across all ledger entries).
    pub entry_id: Uuid,
    /// Warehouse the movement occurred in.
    pub warehouse_id: Uuid,
    /// Location (bin/shelf) if the movement was location-aware; NULL otherwise.
    pub location_id: Option<Uuid>,
    /// Movement type: "received" | "issued" | "adjusted" | "transfer_in" | "transfer_out".
    pub entry_type: String,
    /// Signed quantity: positive = stock in, negative = stock out.
    pub quantity: i64,
    /// Unit cost in minor currency units (e.g. cents).
    pub unit_cost_minor: i64,
    /// ISO 4217 currency code (lower-case, e.g. "usd").
    pub currency: String,
    /// UUID of the outbox event that caused this ledger entry.
    pub source_event_id: Uuid,
    /// Event type string (e.g. "inventory.item_received").
    pub source_event_type: String,
    /// Optional business reference type (e.g. "purchase_order", "sales_order").
    pub reference_type: Option<String>,
    /// Optional business reference ID (e.g. the PO or SO number).
    pub reference_id: Option<String>,
    /// Wall-clock time at which the movement was posted.
    pub posted_at: DateTime<Utc>,
}

// ============================================================================
// Public API
// ============================================================================

/// Return all ledger movements for `(tenant_id, item_id)`, ordered by
/// `posted_at ASC, id ASC`.
///
/// When `location_id` is `Some(l)`, results are narrowed to movements that
/// touched location `l`.  When `None`, all movements across all locations
/// (and NULL-location movements) are returned.
///
/// Returns an empty `Vec` when no entries exist — not an error.
pub async fn query_movement_history(
    pool: &PgPool,
    tenant_id: &str,
    item_id: Uuid,
    location_id: Option<Uuid>,
) -> Result<Vec<MovementEntry>, sqlx::Error> {
    match location_id {
        None => {
            sqlx::query_as::<_, MovementEntry>(
                r#"
                SELECT id              AS ledger_id,
                       entry_id,
                       warehouse_id,
                       location_id,
                       entry_type::TEXT,
                       quantity,
                       unit_cost_minor,
                       currency,
                       source_event_id,
                       source_event_type,
                       reference_type,
                       reference_id,
                       posted_at
                FROM inventory_ledger
                WHERE tenant_id = $1
                  AND item_id   = $2
                ORDER BY posted_at ASC, id ASC
                "#,
            )
            .bind(tenant_id)
            .bind(item_id)
            .fetch_all(pool)
            .await
        }
        Some(loc_id) => {
            sqlx::query_as::<_, MovementEntry>(
                r#"
                SELECT id              AS ledger_id,
                       entry_id,
                       warehouse_id,
                       location_id,
                       entry_type::TEXT,
                       quantity,
                       unit_cost_minor,
                       currency,
                       source_event_id,
                       source_event_type,
                       reference_type,
                       reference_id,
                       posted_at
                FROM inventory_ledger
                WHERE tenant_id  = $1
                  AND item_id    = $2
                  AND location_id = $3
                ORDER BY posted_at ASC, id ASC
                "#,
            )
            .bind(tenant_id)
            .bind(item_id)
            .bind(loc_id)
            .fetch_all(pool)
            .await
        }
    }
}
