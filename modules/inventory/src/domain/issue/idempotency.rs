//! Idempotency key handling, replay logic, and shared DB helpers for stock issues.

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::{domain::lots_serials::issue as ls_issue, events::contracts::ConsumedLayer};

use super::types::{IdempotencyRecord, IssueError, IssueRequest, IssueResult};

/// Check if an idempotency key already exists; return stored result if so.
///
/// Returns `Ok(Some((result, true)))` on replay, `Ok(None)` if no key found.
pub(super) async fn check_idempotency(
    pool: &PgPool,
    req: &IssueRequest,
    request_hash: &str,
) -> Result<Option<(IssueResult, bool)>, IssueError> {
    if let Some(record) = find_idempotency_key(pool, &req.tenant_id, &req.idempotency_key).await? {
        if record.request_hash != request_hash {
            return Err(IssueError::ConflictingIdempotencyKey);
        }
        let result: IssueResult = serde_json::from_str(&record.response_body)?;
        return Ok(Some((result, true)));
    }
    Ok(None)
}

pub(super) async fn find_idempotency_key(
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

/// Aggregate locked serials by layer_id to build `ConsumedLayer` slices.
///
/// Uses BTreeMap to produce deterministic ordering by layer_id.
pub(super) fn build_consumed_from_serials(locked: &[ls_issue::LockedSerial]) -> Vec<ConsumedLayer> {
    let mut by_layer: std::collections::BTreeMap<Uuid, (i64, i64)> = Default::default();
    for s in locked {
        let entry = by_layer.entry(s.layer_id).or_insert((0, s.unit_cost_minor));
        entry.0 += 1;
    }
    by_layer
        .into_iter()
        .map(|(layer_id, (qty, unit_cost))| ConsumedLayer {
            layer_id,
            quantity: qty,
            unit_cost_minor: unit_cost,
            extended_cost_minor: qty * unit_cost,
        })
        .collect()
}

/// Store an idempotency key in the inv_idempotency_keys table.
pub(super) async fn store_idempotency_key(
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

/// Fetch warehouse-level sum(remaining) and sum(remaining * unit_cost) for on-hand projection.
pub(super) async fn fetch_warehouse_totals(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
) -> Result<(i64, i64), sqlx::Error> {
    sqlx::query_as(
        r#"
        SELECT COALESCE(SUM(quantity_remaining), 0)::BIGINT,
               COALESCE(SUM(quantity_remaining * unit_cost_minor), 0)::BIGINT
        FROM inventory_layers
        WHERE tenant_id = $1 AND item_id = $2 AND warehouse_id = $3
          AND quantity_remaining > 0
        "#,
    )
    .bind(tenant_id)
    .bind(item_id)
    .bind(warehouse_id)
    .fetch_one(&mut **tx)
    .await
}

/// Write layer_consumptions rows and decrement layer quantities in a single pass.
pub(super) async fn write_layer_consumptions(
    tx: &mut Transaction<'_, Postgres>,
    consumed: &[ConsumedLayer],
    ledger_id: i64,
    issued_at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    for c in consumed {
        sqlx::query(
            r#"
            INSERT INTO layer_consumptions
                (layer_id, ledger_entry_id, quantity_consumed, unit_cost_minor, consumed_at)
            VALUES ($1, $2, $3, $4, $5)
            "#,
        )
        .bind(c.layer_id)
        .bind(ledger_id)
        .bind(c.quantity)
        .bind(c.unit_cost_minor)
        .bind(issued_at)
        .execute(&mut **tx)
        .await?;

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
        .bind(c.quantity)
        .bind(issued_at)
        .bind(c.layer_id)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_consumed_from_serials_groups_by_layer() {
        let layer_a = Uuid::new_v4();
        let layer_b = Uuid::new_v4();
        let locked = vec![
            ls_issue::LockedSerial {
                serial_id: Uuid::new_v4(),
                layer_id: layer_a,
                unit_cost_minor: 1000,
            },
            ls_issue::LockedSerial {
                serial_id: Uuid::new_v4(),
                layer_id: layer_b,
                unit_cost_minor: 2000,
            },
            ls_issue::LockedSerial {
                serial_id: Uuid::new_v4(),
                layer_id: layer_a,
                unit_cost_minor: 1000,
            },
        ];
        let consumed = build_consumed_from_serials(&locked);
        assert_eq!(consumed.len(), 2);
        let entry_a = consumed
            .iter()
            .find(|c| c.layer_id == layer_a)
            .expect("layer_a");
        assert_eq!(entry_a.quantity, 2);
        assert_eq!(entry_a.extended_cost_minor, 2000);
        let entry_b = consumed
            .iter()
            .find(|c| c.layer_id == layer_b)
            .expect("layer_b");
        assert_eq!(entry_b.quantity, 1);
        assert_eq!(entry_b.extended_cost_minor, 2000);
    }
}
