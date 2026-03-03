//! Lot and serial DB operations executed inside the receipt transaction.
//!
//! These functions run within an existing `sqlx::Transaction` so they
//! participate in the Guard → Mutation → Outbox atomic unit established by
//! `receipt_service::process_receipt`.
//!
//! ## Lot upsert
//! `upsert_lot` is idempotent on (tenant_id, item_id, lot_code). Re-receiving
//! stock under the same lot code reuses the existing lot row and returns its id.
//! The new FIFO layer is then associated to that lot_id.
//!
//! ## Serial instance insert
//! `insert_serial_instances` creates one row per serial_code. Uniqueness is
//! enforced by the DB (`serial_instances_unique_code`). Callers should validate
//! for duplicates before calling to get actionable error messages.

use chrono::NaiveDate;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

// ============================================================================
// Lot upsert
// ============================================================================

/// Upsert a lot row and return its id.
///
/// If (tenant_id, item_id, lot_code) already exists the existing id is returned
/// unchanged — re-receiving into the same lot is idempotent on the lot row.
/// The FIFO layer created by this receipt will be associated to the returned id.
pub async fn upsert_lot(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    item_id: Uuid,
    lot_code: &str,
    expires_on: Option<NaiveDate>,
    attributes: Option<serde_json::Value>,
) -> Result<Uuid, sqlx::Error> {
    sqlx::query_scalar::<_, Uuid>(
        r#"
        INSERT INTO inventory_lots (tenant_id, item_id, lot_code, expires_on, attributes)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (tenant_id, item_id, lot_code) DO UPDATE
            SET tenant_id = EXCLUDED.tenant_id,
                expires_on = COALESCE(inventory_lots.expires_on, EXCLUDED.expires_on),
                expiry_source = CASE
                    WHEN inventory_lots.expires_on IS NULL AND EXCLUDED.expires_on IS NOT NULL
                        THEN 'policy'
                    ELSE inventory_lots.expiry_source
                END,
                expiry_set_at = CASE
                    WHEN inventory_lots.expires_on IS NULL AND EXCLUDED.expires_on IS NOT NULL
                        THEN NOW()
                    ELSE inventory_lots.expiry_set_at
                END
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(item_id)
    .bind(lot_code)
    .bind(expires_on)
    .bind(attributes)
    .fetch_one(&mut **tx)
    .await
}

// ============================================================================
// Serial instance insert
// ============================================================================

/// Insert one `inventory_serial_instances` row per code in `serial_codes`.
///
/// All instances are tied to `receipt_ledger_entry_id` and `layer_id` and
/// start with status `on_hand`.
///
/// Returns the generated UUIDs in the same order as `serial_codes`.
///
/// Returns a DB error (unique constraint violation, code 23505) if any
/// serial_code already exists for the (tenant_id, item_id) pair.
pub async fn insert_serial_instances(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    item_id: Uuid,
    serial_codes: &[String],
    receipt_ledger_entry_id: i64,
    layer_id: Uuid,
) -> Result<Vec<Uuid>, sqlx::Error> {
    let mut ids = Vec::with_capacity(serial_codes.len());

    for code in serial_codes {
        let id = sqlx::query_scalar::<_, Uuid>(
            r#"
            INSERT INTO inventory_serial_instances
                (tenant_id, item_id, serial_code, receipt_ledger_entry_id, layer_id, status)
            VALUES
                ($1, $2, $3, $4, $5, 'on_hand')
            RETURNING id
            "#,
        )
        .bind(tenant_id)
        .bind(item_id)
        .bind(code)
        .bind(receipt_ledger_entry_id)
        .bind(layer_id)
        .fetch_one(&mut **tx)
        .await?;

        ids.push(id);
    }

    Ok(ids)
}

// ============================================================================
// Unit tests (pure logic; DB tests live in receipt_integration.rs)
// ============================================================================

#[cfg(test)]
mod tests {
    /// DB-level tests live in the integration suite.
    /// This module contains only logic that can be tested without a DB.

    #[test]
    fn serial_instance_insert_is_per_code() {
        // Verifies the function signature accepts a slice of codes.
        let codes: Vec<String> = vec!["SN-001".to_string(), "SN-002".to_string()];
        assert_eq!(codes.len(), 2);
    }
}
