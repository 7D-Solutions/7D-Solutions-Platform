//! Lot and serial DB operations for issue transactions.
//!
//! These functions run within an existing sqlx::Transaction or against a pool
//! for read-only lookups. They participate in the Guard → Mutation → Outbox
//! atomic unit established by `issue_service::process_issue`.
//!
//! ## Lot-tracked issues
//! `find_lot_id` resolves a lot_code to its UUID so the FIFO layer lock query
//! can filter to only the layers belonging to that lot.
//!
//! ## Serial-tracked issues
//! `validate_and_lock_serials` checks every serial_code is `on_hand` and locks
//! both the serial instance rows and their FIFO layers FOR UPDATE, preventing
//! concurrent double-issue of the same unit.
//! `mark_serials_issued` atomically sets status = 'issued' for all locked serials.

use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

// ============================================================================
// Types
// ============================================================================

/// Per-serial info returned from validate_and_lock_serials.
#[derive(Debug)]
pub struct LockedSerial {
    pub serial_id: Uuid,
    pub layer_id: Uuid,
    pub unit_cost_minor: i64,
}

/// Errors from lot/serial issue validation.
#[derive(Debug)]
pub enum LotSerialError {
    /// A serial code was not found or is not on_hand.
    SerialNotAvailable(String),
    /// DB error.
    Database(sqlx::Error),
}

impl From<sqlx::Error> for LotSerialError {
    fn from(e: sqlx::Error) -> Self {
        LotSerialError::Database(e)
    }
}

// ============================================================================
// Lot helpers
// ============================================================================

/// Look up the UUID of a lot by (tenant, item, lot_code).
///
/// Returns `None` when the lot does not exist. This is a read-only query
/// executed against the pool (not a transaction) since lots are immutable once
/// created.
pub async fn find_lot_id(
    pool: &PgPool,
    tenant_id: &str,
    item_id: Uuid,
    lot_code: &str,
) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar::<_, Uuid>(
        r#"
        SELECT id FROM inventory_lots
        WHERE tenant_id = $1 AND item_id = $2 AND lot_code = $3
        "#,
    )
    .bind(tenant_id)
    .bind(item_id)
    .bind(lot_code)
    .fetch_optional(pool)
    .await
}

// ============================================================================
// Serial helpers
// ============================================================================

#[derive(sqlx::FromRow)]
struct SerialLockRow {
    serial_id: Uuid,
    serial_code: String,
    layer_id: Uuid,
    unit_cost_minor: i64,
}

/// Validate that every serial_code in `serial_codes` is on_hand for (tenant, item),
/// and lock both serial instance rows and their FIFO layers FOR UPDATE.
///
/// The lock prevents concurrent transactions from issuing the same unit.
///
/// Returns `LotSerialError::SerialNotAvailable(code)` if any code is not found
/// or not in `on_hand` status. All codes must be valid for the call to succeed.
pub async fn validate_and_lock_serials(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    item_id: Uuid,
    serial_codes: &[String],
) -> Result<Vec<LockedSerial>, LotSerialError> {
    let rows = sqlx::query_as::<_, SerialLockRow>(
        r#"
        SELECT si.id   AS serial_id,
               si.serial_code,
               l.id    AS layer_id,
               l.unit_cost_minor
        FROM inventory_serial_instances si
        INNER JOIN inventory_layers l ON l.id = si.layer_id
        WHERE si.tenant_id   = $1
          AND si.item_id     = $2
          AND si.serial_code = ANY($3)
          AND si.status      = 'on_hand'
        ORDER BY si.id ASC
        FOR UPDATE OF si, l
        "#,
    )
    .bind(tenant_id)
    .bind(item_id)
    .bind(serial_codes)
    .fetch_all(&mut **tx)
    .await?;

    // All requested codes must be found on_hand.
    if rows.len() != serial_codes.len() {
        let found: std::collections::HashSet<&str> =
            rows.iter().map(|r| r.serial_code.as_str()).collect();
        let unavailable = serial_codes
            .iter()
            .find(|c| !found.contains(c.as_str()))
            .cloned()
            .unwrap_or_else(|| "unknown".to_string());
        return Err(LotSerialError::SerialNotAvailable(unavailable));
    }

    Ok(rows
        .into_iter()
        .map(|r| LockedSerial {
            serial_id: r.serial_id,
            layer_id: r.layer_id,
            unit_cost_minor: r.unit_cost_minor,
        })
        .collect())
}

/// Mark all given serial instance IDs as `issued` within the transaction.
///
/// Must be called after all ledger rows and layer_consumptions have been
/// written so the update participates in the same commit.
pub async fn mark_serials_issued(
    tx: &mut Transaction<'_, Postgres>,
    serial_ids: &[Uuid],
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE inventory_serial_instances
        SET status = 'issued'
        WHERE id = ANY($1)
        "#,
    )
    .bind(serial_ids)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

// ============================================================================
// Tests (pure logic; DB tests live in integration suite)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locked_serial_struct_is_constructable() {
        let _ = LockedSerial {
            serial_id: Uuid::new_v4(),
            layer_id: Uuid::new_v4(),
            unit_cost_minor: 1000,
        };
    }

    #[test]
    fn lot_serial_error_debug() {
        let e = LotSerialError::SerialNotAvailable("SN-001".to_string());
        let s = format!("{:?}", e);
        assert!(s.contains("SN-001"));
    }
}
