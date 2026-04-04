//! Cycle count repository — database operations for cycle count approval.
//!
//! All transactional functions accept `&mut Transaction<'_, Postgres>` to
//! participate in the Guard → Mutation → Outbox atomic unit orchestrated by
//! `approve_service`.

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
pub(crate) struct TaskRow {
    pub status: String,
    pub warehouse_id: Uuid,
    pub location_id: Uuid,
}

#[derive(sqlx::FromRow)]
pub(crate) struct LineWithSku {
    pub id: Uuid,
    pub item_id: Uuid,
    pub expected_qty: i64,
    pub counted_qty: Option<i64>,
    pub sku: String,
}

#[derive(sqlx::FromRow)]
pub(crate) struct LedgerInserted {
    pub id: i64,
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
// Guard reads
// ============================================================================

/// Load the cycle count task row. Returns None if not found for tenant.
pub(crate) async fn fetch_task(
    pool: &PgPool,
    task_id: Uuid,
    tenant_id: &str,
) -> Result<Option<TaskRow>, sqlx::Error> {
    sqlx::query_as::<_, TaskRow>(
        r#"
        SELECT status::TEXT AS status, warehouse_id, location_id
        FROM cycle_count_tasks
        WHERE id = $1 AND tenant_id = $2
        "#,
    )
    .bind(task_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await
}

/// Load all lines for a task, joined with item SKU.
pub(crate) async fn fetch_lines_with_sku(
    pool: &PgPool,
    task_id: Uuid,
) -> Result<Vec<LineWithSku>, sqlx::Error> {
    sqlx::query_as::<_, LineWithSku>(
        r#"
        SELECT ccl.id, ccl.item_id, ccl.expected_qty, ccl.counted_qty, i.sku
        FROM cycle_count_lines ccl
        JOIN items i ON i.id = ccl.item_id
        WHERE ccl.task_id = $1
        "#,
    )
    .bind(task_id)
    .fetch_all(pool)
    .await
}

// ============================================================================
// Adjustment mutations
// ============================================================================

/// Insert an 'adjusted' ledger row. Returns the ledger id.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn insert_ledger_row(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
    location_id: Uuid,
    variance_qty: i64,
    adj_event_id: Uuid,
    event_type: &str,
    approved_at: DateTime<Utc>,
) -> Result<i64, sqlx::Error> {
    let row = sqlx::query_as::<_, LedgerInserted>(
        r#"
        INSERT INTO inventory_ledger
            (tenant_id, item_id, warehouse_id, location_id, entry_type,
             quantity, unit_cost_minor, currency,
             source_event_id, source_event_type,
             reference_type, notes, posted_at)
        VALUES
            ($1, $2, $3, $4, 'adjusted', $5, 0, 'usd', $6, $7,
             'adjustment', 'cycle_count', $8)
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(item_id)
    .bind(warehouse_id)
    .bind(location_id)
    .bind(variance_qty)
    .bind(adj_event_id)
    .bind(event_type)
    .bind(approved_at)
    .fetch_one(&mut **tx)
    .await?;
    Ok(row.id)
}

/// Insert an inv_adjustments business-key row. Returns the adjustment id.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn insert_adjustment(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
    location_id: Uuid,
    variance_qty: i64,
    adj_event_id: Uuid,
    ledger_entry_id: i64,
    approved_at: DateTime<Utc>,
) -> Result<Uuid, sqlx::Error> {
    sqlx::query_scalar::<_, Uuid>(
        r#"
        INSERT INTO inv_adjustments
            (tenant_id, item_id, warehouse_id, location_id,
             quantity_delta, reason, event_id, ledger_entry_id, adjusted_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(item_id)
    .bind(warehouse_id)
    .bind(location_id)
    .bind(variance_qty)
    .bind("cycle_count")
    .bind(adj_event_id)
    .bind(ledger_entry_id)
    .bind(approved_at)
    .fetch_one(&mut **tx)
    .await
}

/// Upsert item_on_hand projection for a location-specific row (delta-based).
pub(crate) async fn upsert_on_hand(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
    location_id: Uuid,
    delta: i64,
    ledger_entry_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO item_on_hand
            (tenant_id, item_id, warehouse_id, location_id,
             quantity_on_hand, available_status_on_hand,
             total_cost_minor, currency, last_ledger_entry_id, projected_at)
        VALUES ($1, $2, $3, $4, $5, $5, 0, 'usd', $6, NOW())
        ON CONFLICT (tenant_id, item_id, warehouse_id, location_id)
        WHERE location_id IS NOT NULL
        DO UPDATE
            SET quantity_on_hand         = item_on_hand.quantity_on_hand + $5,
                available_status_on_hand = item_on_hand.available_status_on_hand + $5,
                last_ledger_entry_id     = $6,
                projected_at             = NOW()
        "#,
    )
    .bind(tenant_id)
    .bind(item_id)
    .bind(warehouse_id)
    .bind(location_id)
    .bind(delta)
    .bind(ledger_entry_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Update item_on_hand_by_status (available bucket) after a cycle count adjustment.
///
/// Positive delta: upsert (create or increment).
/// Negative delta: UPDATE only to avoid CHECK constraint violation on INSERT.
pub(crate) async fn upsert_available_bucket(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
    delta: i64,
) -> Result<(), sqlx::Error> {
    if delta >= 0 {
        sqlx::query(
            r#"
            INSERT INTO item_on_hand_by_status
                (tenant_id, item_id, warehouse_id, status, quantity_on_hand)
            VALUES ($1, $2, $3, 'available', $4)
            ON CONFLICT (tenant_id, item_id, warehouse_id, status) DO UPDATE
                SET quantity_on_hand = item_on_hand_by_status.quantity_on_hand + $4,
                    updated_at       = NOW()
            "#,
        )
        .bind(tenant_id)
        .bind(item_id)
        .bind(warehouse_id)
        .bind(delta)
        .execute(&mut **tx)
        .await?;
    } else {
        sqlx::query(
            r#"
            UPDATE item_on_hand_by_status
            SET quantity_on_hand = GREATEST(0, quantity_on_hand + $4),
                updated_at       = NOW()
            WHERE tenant_id    = $1
              AND item_id      = $2
              AND warehouse_id = $3
              AND status       = 'available'
            "#,
        )
        .bind(tenant_id)
        .bind(item_id)
        .bind(warehouse_id)
        .bind(delta)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

/// Move task to 'approved' status.
pub(crate) async fn update_task_approved(
    tx: &mut Transaction<'_, Postgres>,
    approved_at: DateTime<Utc>,
    task_id: Uuid,
    tenant_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE cycle_count_tasks
        SET status = 'approved', updated_at = $1
        WHERE id = $2 AND tenant_id = $3
        "#,
    )
    .bind(approved_at)
    .bind(task_id)
    .bind(tenant_id)
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
