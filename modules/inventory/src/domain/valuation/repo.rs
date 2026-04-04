//! Valuation repository — database operations shared by run_service and snapshot_service.
//!
//! Run and snapshot use distinct layer row types (different query shapes) so they
//! are namespaced with `Run` / `Snapshot` prefixes where needed.

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction};
use std::collections::BTreeMap;
use uuid::Uuid;

// ============================================================================
// Internal DB row types
// ============================================================================

#[derive(sqlx::FromRow)]
pub(crate) struct IdempotencyRecord {
    pub response_body: String,
    pub request_hash: String,
}

/// Layer row for valuation runs (includes quantity_received and qty_consumed_at_as_of).
#[derive(sqlx::FromRow)]
pub(crate) struct RunLayerRow {
    pub item_id: Uuid,
    pub unit_cost_minor: i64,
    pub quantity_received: i64,
    pub qty_consumed_at_as_of: i64,
}

/// Layer row for valuation snapshots (only qty remaining at as_of).
#[derive(sqlx::FromRow)]
pub(crate) struct SnapshotLayerRow {
    pub item_id: Uuid,
    pub unit_cost_minor: i64,
    pub qty_at_as_of: i64,
}

#[derive(sqlx::FromRow)]
pub(crate) struct StandardCostConfig {
    pub item_id: Uuid,
    pub standard_cost_minor: Option<i64>,
}

// ============================================================================
// Shared idempotency
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
// Shared advisory lock
// ============================================================================

/// Try to acquire a per-transaction advisory lock. Returns false if already held.
pub(crate) async fn try_advisory_lock(
    tx: &mut Transaction<'_, Postgres>,
    lock_key: i64,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT pg_try_advisory_xact_lock($1)")
        .bind(lock_key)
        .fetch_one(&mut **tx)
        .await
}

// ============================================================================
// Valuation run queries
// ============================================================================

/// Fetch FIFO layer data for all items in the warehouse up to as_of.
pub(crate) async fn fetch_layers_for_run(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    warehouse_id: Uuid,
    as_of: DateTime<Utc>,
) -> Result<Vec<RunLayerRow>, sqlx::Error> {
    sqlx::query_as::<_, RunLayerRow>(
        r#"
        SELECT
            l.item_id,
            l.unit_cost_minor,
            l.quantity_received,
            COALESCE(
                SUM(lc.quantity_consumed) FILTER (WHERE lc.consumed_at <= $3),
                0
            )::BIGINT AS qty_consumed_at_as_of
        FROM inventory_layers l
        LEFT JOIN layer_consumptions lc ON lc.layer_id = l.id
        WHERE l.tenant_id = $1
          AND l.warehouse_id = $2
          AND l.received_at <= $3
        GROUP BY l.id, l.item_id, l.unit_cost_minor, l.quantity_received, l.received_at
        ORDER BY l.item_id, l.received_at ASC, l.id
        "#,
    )
    .bind(tenant_id)
    .bind(warehouse_id)
    .bind(as_of)
    .fetch_all(&mut **tx)
    .await
}

/// Load standard cost configs for all items with method = 'standard_cost'.
pub(crate) async fn load_standard_costs(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
) -> Result<BTreeMap<Uuid, i64>, sqlx::Error> {
    let rows: Vec<StandardCostConfig> = sqlx::query_as::<_, StandardCostConfig>(
        r#"
        SELECT item_id, standard_cost_minor
        FROM item_valuation_configs
        WHERE tenant_id = $1 AND method = 'standard_cost'
        "#,
    )
    .bind(tenant_id)
    .fetch_all(&mut **tx)
    .await?;

    Ok(rows
        .into_iter()
        .filter_map(|r| r.standard_cost_minor.map(|c| (r.item_id, c)))
        .collect())
}

/// Insert the valuation run header row.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn insert_run_header(
    tx: &mut Transaction<'_, Postgres>,
    run_id: Uuid,
    tenant_id: &str,
    warehouse_id: Uuid,
    method: &str,
    as_of: DateTime<Utc>,
    total_value_minor: i64,
    currency: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO valuation_runs
            (id, tenant_id, warehouse_id, method, as_of,
             total_value_minor, total_cogs_minor, currency)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        "#,
    )
    .bind(run_id)
    .bind(tenant_id)
    .bind(warehouse_id)
    .bind(method)
    .bind(as_of)
    .bind(total_value_minor)
    .bind(0_i64)
    .bind(currency)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Batch insert per-item lines for a valuation run.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn insert_run_lines(
    tx: &mut Transaction<'_, Postgres>,
    run_id: Uuid,
    item_ids: &[Uuid],
    warehouse_ids: &[Uuid],
    qtys: &[i64],
    unit_costs: &[i64],
    total_values: &[i64],
    variances: &[i64],
    currencies: &[&str],
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO valuation_run_lines
            (run_id, item_id, warehouse_id,
             quantity_on_hand, unit_cost_minor, total_value_minor,
             variance_minor, currency)
        SELECT $1,
            UNNEST($2::UUID[]),
            UNNEST($3::UUID[]),
            UNNEST($4::BIGINT[]),
            UNNEST($5::BIGINT[]),
            UNNEST($6::BIGINT[]),
            UNNEST($7::BIGINT[]),
            UNNEST($8::TEXT[])
        "#,
    )
    .bind(run_id)
    .bind(item_ids)
    .bind(warehouse_ids)
    .bind(qtys)
    .bind(unit_costs)
    .bind(total_values)
    .bind(variances)
    .bind(currencies)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub(crate) async fn insert_run_outbox_event(
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
            ($1, $2, 'valuation_run', $3, $4, $5::JSONB, $6, $7, '1.0.0')
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

/// Upsert the per-item valuation method configuration.
pub(crate) async fn upsert_valuation_method(
    pool: &PgPool,
    tenant_id: &str,
    item_id: Uuid,
    method: &str,
    standard_cost_minor: Option<i64>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO item_valuation_configs
            (tenant_id, item_id, method, standard_cost_minor)
        VALUES ($1, $2, $3, $4)
        ON CONFLICT (tenant_id, item_id) DO UPDATE SET
            method = EXCLUDED.method,
            standard_cost_minor = EXCLUDED.standard_cost_minor,
            updated_at = NOW()
        "#,
    )
    .bind(tenant_id)
    .bind(item_id)
    .bind(method)
    .bind(standard_cost_minor)
    .execute(pool)
    .await?;
    Ok(())
}

// ============================================================================
// Valuation snapshot queries
// ============================================================================

/// Fetch FIFO layer state at as_of for snapshot computation.
pub(crate) async fn fetch_layers_for_snapshot(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    warehouse_id: Uuid,
    as_of: DateTime<Utc>,
) -> Result<Vec<SnapshotLayerRow>, sqlx::Error> {
    sqlx::query_as::<_, SnapshotLayerRow>(
        r#"
        SELECT
            l.item_id,
            l.unit_cost_minor,
            (l.quantity_received - COALESCE(
                SUM(lc.quantity_consumed) FILTER (WHERE lc.consumed_at <= $3),
                0
            ))::BIGINT AS qty_at_as_of
        FROM inventory_layers l
        LEFT JOIN layer_consumptions lc ON lc.layer_id = l.id
        WHERE l.tenant_id = $1
          AND l.warehouse_id = $2
          AND l.received_at <= $3
        GROUP BY l.id, l.item_id, l.unit_cost_minor, l.quantity_received
        HAVING (l.quantity_received - COALESCE(
            SUM(lc.quantity_consumed) FILTER (WHERE lc.consumed_at <= $3),
            0
        )) > 0
        "#,
    )
    .bind(tenant_id)
    .bind(warehouse_id)
    .bind(as_of)
    .fetch_all(&mut **tx)
    .await
}

/// Insert the snapshot header row.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn insert_snapshot_header(
    tx: &mut Transaction<'_, Postgres>,
    snapshot_id: Uuid,
    tenant_id: &str,
    warehouse_id: Uuid,
    location_id: Option<Uuid>,
    as_of: DateTime<Utc>,
    total_value_minor: i64,
    currency: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO inventory_valuation_snapshots
            (id, tenant_id, warehouse_id, location_id, as_of,
             total_value_minor, currency)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        "#,
    )
    .bind(snapshot_id)
    .bind(tenant_id)
    .bind(warehouse_id)
    .bind(location_id)
    .bind(as_of)
    .bind(total_value_minor)
    .bind(currency)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Batch insert per-item lines for a snapshot.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn insert_snapshot_lines(
    tx: &mut Transaction<'_, Postgres>,
    snapshot_id: Uuid,
    item_ids: &[Uuid],
    warehouse_ids: &[Uuid],
    location_ids: &[Option<Uuid>],
    qtys: &[i64],
    unit_costs: &[i64],
    total_values: &[i64],
    currencies: &[&str],
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO inventory_valuation_lines
            (snapshot_id, item_id, warehouse_id, location_id,
             quantity_on_hand, unit_cost_minor, total_value_minor, currency)
        SELECT $1,
            UNNEST($2::UUID[]),
            UNNEST($3::UUID[]),
            UNNEST($4::UUID[]),
            UNNEST($5::BIGINT[]),
            UNNEST($6::BIGINT[]),
            UNNEST($7::BIGINT[]),
            UNNEST($8::TEXT[])
        "#,
    )
    .bind(snapshot_id)
    .bind(item_ids)
    .bind(warehouse_ids)
    .bind(location_ids)
    .bind(qtys)
    .bind(unit_costs)
    .bind(total_values)
    .bind(currencies)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub(crate) async fn insert_snapshot_outbox_event(
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
            ($1, $2, 'valuation_snapshot', $3, $4, $5::JSONB, $6, $7, '1.0.0')
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
