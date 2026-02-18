//! Valuation snapshot builder.
//!
//! Creates a point-in-time valuation snapshot from FIFO layer state as-of a
//! given timestamp. Uses a per-tenant advisory lock to prevent concurrent
//! snapshot writers racing on the same tenant's inventory.
//!
//! Pattern: Guard → Mutation → Outbox (all in one transaction)
//!
//! Guards:
//!   - tenant_id must be non-empty
//!   - idempotency_key must be non-empty
//!   - per-tenant advisory lock acquired (fails fast if already held)
//!
//! Value computation per item:
//!   qty_at_as_of  = quantity_received − SUM(consumption.qty WHERE consumed_at ≤ as_of)
//!   total_value   = SUM(qty_at_as_of × unit_cost_minor) across all remaining layers
//!   unit_cost_avg = total_value / total_qty (weighted average)

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::collections::HashMap;
use thiserror::Error;
use uuid::Uuid;

use crate::events::valuation_snapshot_created::{
    build_valuation_snapshot_created_envelope, ValuationSnapshotCreatedLine,
    ValuationSnapshotCreatedPayload, EVENT_TYPE_VALUATION_SNAPSHOT_CREATED,
};

// ============================================================================
// Types
// ============================================================================

/// Request to build a valuation snapshot.
#[derive(Debug, Serialize, Deserialize)]
pub struct CreateSnapshotRequest {
    pub tenant_id: String,
    pub warehouse_id: Uuid,
    /// None = warehouse-level roll-up; Some = location-scoped header
    pub location_id: Option<Uuid>,
    /// Point-in-time for FIFO layer evaluation
    pub as_of: DateTime<Utc>,
    pub idempotency_key: String,
    pub currency: String,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
}

/// One per-item line in the snapshot result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotLineResult {
    pub item_id: Uuid,
    pub warehouse_id: Uuid,
    pub location_id: Option<Uuid>,
    pub quantity_on_hand: i64,
    /// Weighted-average unit cost across remaining FIFO layers
    pub unit_cost_minor: i64,
    /// quantity_on_hand × unit_cost_minor
    pub total_value_minor: i64,
    pub currency: String,
}

/// Returned on successful or replayed snapshot creation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotResult {
    pub snapshot_id: Uuid,
    pub tenant_id: String,
    pub warehouse_id: Uuid,
    pub location_id: Option<Uuid>,
    pub as_of: DateTime<Utc>,
    pub total_value_minor: i64,
    pub currency: String,
    pub line_count: usize,
    pub lines: Vec<SnapshotLineResult>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Error)]
pub enum SnapshotError {
    #[error("tenant_id is required")]
    MissingTenant,

    #[error("idempotency_key is required")]
    MissingIdempotencyKey,

    #[error("concurrent snapshot already in progress for this tenant; retry later")]
    ConcurrentSnapshot,

    #[error("idempotency key conflict: same key used with a different request body")]
    ConflictingIdempotencyKey,

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ============================================================================
// Internal row types
// ============================================================================

#[derive(sqlx::FromRow)]
struct LayerRow {
    item_id: Uuid,
    unit_cost_minor: i64,
    qty_at_as_of: i64,
}

#[derive(sqlx::FromRow)]
struct IdempotencyRecord {
    response_body: String,
    request_hash: String,
}

// ============================================================================
// Service
// ============================================================================

/// Build a valuation snapshot deterministically from FIFO layer state.
///
/// Returns `(SnapshotResult, is_replay)`:
/// - `is_replay = false`: snapshot created; HTTP 201.
/// - `is_replay = true`:  idempotency hit; HTTP 200 with stored result.
pub async fn create_valuation_snapshot(
    pool: &PgPool,
    req: &CreateSnapshotRequest,
) -> Result<(SnapshotResult, bool), SnapshotError> {
    validate_request(req)?;

    let request_hash = serde_json::to_string(req)?;
    if let Some(record) =
        find_idempotency_key(pool, &req.tenant_id, &req.idempotency_key).await?
    {
        if record.request_hash != request_hash {
            return Err(SnapshotError::ConflictingIdempotencyKey);
        }
        let result: SnapshotResult = serde_json::from_str(&record.response_body)?;
        return Ok((result, true));
    }

    let created_at = Utc::now();
    let event_id = Uuid::new_v4();
    let snapshot_id = Uuid::new_v4();
    let correlation_id = req
        .correlation_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let mut tx = pool.begin().await?;

    // --- Advisory lock: one snapshot writer per tenant at a time ---
    let lock_key = fnv_key(&req.tenant_id);
    let acquired: bool =
        sqlx::query_scalar("SELECT pg_try_advisory_xact_lock($1)")
            .bind(lock_key)
            .fetch_one(&mut *tx)
            .await?;
    if !acquired {
        return Err(SnapshotError::ConcurrentSnapshot);
    }

    // --- Query FIFO layer state at as_of (warehouse-level) ---
    let layer_rows: Vec<LayerRow> = sqlx::query_as::<_, LayerRow>(
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
    .bind(&req.tenant_id)
    .bind(req.warehouse_id)
    .bind(req.as_of)
    .fetch_all(&mut *tx)
    .await?;

    // --- Aggregate by item: weighted-average cost ---
    let lines =
        aggregate_lines(&layer_rows, req.warehouse_id, req.location_id, &req.currency);
    let total_value_minor: i64 = lines.iter().map(|l| l.total_value_minor).sum();

    // --- Mutation: insert snapshot header ---
    sqlx::query(
        r#"
        INSERT INTO inventory_valuation_snapshots
            (id, tenant_id, warehouse_id, location_id, as_of,
             total_value_minor, currency)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        "#,
    )
    .bind(snapshot_id)
    .bind(&req.tenant_id)
    .bind(req.warehouse_id)
    .bind(req.location_id)
    .bind(req.as_of)
    .bind(total_value_minor)
    .bind(&req.currency)
    .execute(&mut *tx)
    .await?;

    // --- Mutation: insert per-item lines ---
    for line in &lines {
        sqlx::query(
            r#"
            INSERT INTO inventory_valuation_lines
                (snapshot_id, item_id, warehouse_id, location_id,
                 quantity_on_hand, unit_cost_minor, total_value_minor, currency)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            "#,
        )
        .bind(snapshot_id)
        .bind(line.item_id)
        .bind(line.warehouse_id)
        .bind(line.location_id)
        .bind(line.quantity_on_hand)
        .bind(line.unit_cost_minor)
        .bind(line.total_value_minor)
        .bind(&line.currency)
        .execute(&mut *tx)
        .await?;
    }

    // --- Outbox: emit inventory.valuation_snapshot_created ---
    let event_lines: Vec<ValuationSnapshotCreatedLine> = lines
        .iter()
        .map(|l| ValuationSnapshotCreatedLine {
            item_id: l.item_id,
            quantity_on_hand: l.quantity_on_hand,
            unit_cost_minor: l.unit_cost_minor,
            total_value_minor: l.total_value_minor,
        })
        .collect();

    let payload = ValuationSnapshotCreatedPayload {
        snapshot_id,
        tenant_id: req.tenant_id.clone(),
        warehouse_id: req.warehouse_id,
        location_id: req.location_id,
        as_of: req.as_of,
        total_value_minor,
        currency: req.currency.clone(),
        line_count: lines.len(),
        lines: event_lines,
    };
    let envelope = build_valuation_snapshot_created_envelope(
        event_id,
        req.tenant_id.clone(),
        correlation_id.clone(),
        req.causation_id.clone(),
        payload,
    );
    let envelope_json = serde_json::to_string(&envelope)?;

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
    .bind(EVENT_TYPE_VALUATION_SNAPSHOT_CREATED)
    .bind(snapshot_id.to_string())
    .bind(&req.tenant_id)
    .bind(&envelope_json)
    .bind(&correlation_id)
    .bind(&req.causation_id)
    .execute(&mut *tx)
    .await?;

    // --- Idempotency: store response for replay ---
    let result = SnapshotResult {
        snapshot_id,
        tenant_id: req.tenant_id.clone(),
        warehouse_id: req.warehouse_id,
        location_id: req.location_id,
        as_of: req.as_of,
        total_value_minor,
        currency: req.currency.clone(),
        line_count: lines.len(),
        lines,
        created_at,
    };
    let response_json = serde_json::to_string(&result)?;
    let expires_at = created_at + Duration::days(7);

    sqlx::query(
        r#"
        INSERT INTO inv_idempotency_keys
            (tenant_id, idempotency_key, request_hash, response_body, status_code, expires_at)
        VALUES ($1, $2, $3, $4::JSONB, 201, $5)
        "#,
    )
    .bind(&req.tenant_id)
    .bind(&req.idempotency_key)
    .bind(&request_hash)
    .bind(&response_json)
    .bind(expires_at)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok((result, false))
}

// ============================================================================
// Helpers
// ============================================================================

fn validate_request(req: &CreateSnapshotRequest) -> Result<(), SnapshotError> {
    if req.tenant_id.trim().is_empty() {
        return Err(SnapshotError::MissingTenant);
    }
    if req.idempotency_key.trim().is_empty() {
        return Err(SnapshotError::MissingIdempotencyKey);
    }
    Ok(())
}

async fn find_idempotency_key(
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

/// Aggregate per-FIFO-layer rows into per-item snapshot lines.
///
/// Uses weighted-average cost: total_value / total_qty.
/// Rows are sorted by item_id for deterministic ordering.
fn aggregate_lines(
    layer_rows: &[LayerRow],
    warehouse_id: Uuid,
    location_id: Option<Uuid>,
    currency: &str,
) -> Vec<SnapshotLineResult> {
    // (total_qty, total_cost) per item
    let mut agg: HashMap<Uuid, (i64, i64)> = HashMap::new();
    for row in layer_rows {
        let e = agg.entry(row.item_id).or_insert((0, 0));
        e.0 += row.qty_at_as_of;
        e.1 += row.qty_at_as_of * row.unit_cost_minor;
    }

    let mut lines: Vec<SnapshotLineResult> = agg
        .into_iter()
        .filter(|(_, (qty, _))| *qty > 0)
        .map(|(item_id, (qty, total_cost))| {
            let unit_cost_minor = if qty > 0 { total_cost / qty } else { 0 };
            SnapshotLineResult {
                item_id,
                warehouse_id,
                location_id,
                quantity_on_hand: qty,
                unit_cost_minor,
                total_value_minor: total_cost,
                currency: currency.to_string(),
            }
        })
        .collect();

    lines.sort_by_key(|l| l.item_id);
    lines
}

/// Stable i64 advisory lock key from a string (FNV-1a hash).
fn fnv_key(s: &str) -> i64 {
    let mut hash: u64 = 14_695_981_039_346_656_037;
    for byte in s.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(1_099_511_628_211);
    }
    hash as i64
}

// ============================================================================
// Unit tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_layer(item_id: Uuid, qty: i64, cost: i64) -> LayerRow {
        LayerRow {
            item_id,
            unit_cost_minor: cost,
            qty_at_as_of: qty,
        }
    }

    #[test]
    fn validate_rejects_empty_tenant() {
        let req = CreateSnapshotRequest {
            tenant_id: "".to_string(),
            warehouse_id: Uuid::new_v4(),
            location_id: None,
            as_of: Utc::now(),
            idempotency_key: "k1".to_string(),
            currency: "usd".to_string(),
            correlation_id: None,
            causation_id: None,
        };
        assert!(matches!(validate_request(&req), Err(SnapshotError::MissingTenant)));
    }

    #[test]
    fn validate_rejects_empty_idempotency_key() {
        let req = CreateSnapshotRequest {
            tenant_id: "t1".to_string(),
            warehouse_id: Uuid::new_v4(),
            location_id: None,
            as_of: Utc::now(),
            idempotency_key: " ".to_string(),
            currency: "usd".to_string(),
            correlation_id: None,
            causation_id: None,
        };
        assert!(matches!(
            validate_request(&req),
            Err(SnapshotError::MissingIdempotencyKey)
        ));
    }

    #[test]
    fn aggregate_lines_computes_weighted_average() {
        let item_id = Uuid::new_v4();
        let wh = Uuid::new_v4();
        let rows = vec![
            make_layer(item_id, 10, 100), // 10 units @ $1.00
            make_layer(item_id, 5, 200),  // 5 units @ $2.00
        ];
        let lines = aggregate_lines(&rows, wh, None, "usd");
        assert_eq!(lines.len(), 1);
        let l = &lines[0];
        assert_eq!(l.quantity_on_hand, 15);
        assert_eq!(l.total_value_minor, 2000); // 10*100 + 5*200
        assert_eq!(l.unit_cost_minor, 133);    // 2000 / 15 (integer division)
    }

    #[test]
    fn aggregate_lines_two_items() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let wh = Uuid::new_v4();
        let rows = vec![make_layer(a, 20, 500), make_layer(b, 10, 1000)];
        let lines = aggregate_lines(&rows, wh, None, "usd");
        assert_eq!(lines.len(), 2);
        let total: i64 = lines.iter().map(|l| l.total_value_minor).sum();
        assert_eq!(total, 20_000); // 20*500 + 10*1000
    }

    #[test]
    fn aggregate_lines_empty_returns_empty() {
        let lines = aggregate_lines(&[], Uuid::new_v4(), None, "usd");
        assert!(lines.is_empty());
    }

    #[test]
    fn fnv_key_is_deterministic() {
        assert_eq!(fnv_key("tenant-1"), fnv_key("tenant-1"));
        assert_ne!(fnv_key("tenant-1"), fnv_key("tenant-2"));
    }
}
