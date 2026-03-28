//! Valuation run service.
//!
//! Executes a point-in-time valuation of inventory under a specified method
//! (FIFO, LIFO, WAC, standard cost). Produces deterministic results from
//! the same inputs.
//!
//! Pattern: Guard → Mutation → Outbox (all in one transaction)
//!
//! Guards:
//!   - tenant_id must be non-empty
//!   - idempotency_key must be non-empty
//!   - method must be valid
//!   - per-tenant advisory lock (prevents concurrent runs)
//!
//! For standard cost: items without a configured standard_cost_minor are
//! skipped (they produce no line in the run).

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::collections::BTreeMap;
use thiserror::Error;
use uuid::Uuid;

use super::methods::{self, FullLayer, ItemValuation, ValuationMethod};
use crate::events::valuation_run_completed::{
    build_valuation_run_completed_envelope, ValuationRunCompletedLine,
    ValuationRunCompletedPayload, EVENT_TYPE_VALUATION_RUN_COMPLETED,
};

// ============================================================================
// Types
// ============================================================================

/// Request to execute a valuation run.
#[derive(Debug, Serialize, Deserialize)]
pub struct ValuationRunRequest {
    pub tenant_id: String,
    pub warehouse_id: Uuid,
    pub method: ValuationMethod,
    pub as_of: DateTime<Utc>,
    pub idempotency_key: String,
    pub currency: String,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
}

/// Result of a valuation run line.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunLineResult {
    pub item_id: Uuid,
    pub warehouse_id: Uuid,
    pub quantity_on_hand: i64,
    pub unit_cost_minor: i64,
    pub total_value_minor: i64,
    pub variance_minor: i64,
    pub currency: String,
}

/// Result of a completed valuation run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValuationRunResult {
    pub run_id: Uuid,
    pub tenant_id: String,
    pub warehouse_id: Uuid,
    pub method: String,
    pub as_of: DateTime<Utc>,
    pub total_value_minor: i64,
    pub total_cogs_minor: i64,
    pub currency: String,
    pub line_count: usize,
    pub lines: Vec<RunLineResult>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Error)]
pub enum RunError {
    #[error("tenant_id is required")]
    MissingTenant,

    #[error("idempotency_key is required")]
    MissingIdempotencyKey,

    #[error("concurrent valuation run already in progress for this tenant")]
    ConcurrentRun,

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
    quantity_received: i64,
    qty_consumed_at_as_of: i64,
}

/// Standard cost config loaded from item_valuation_configs.
#[derive(sqlx::FromRow)]
struct StandardCostConfig {
    item_id: Uuid,
    standard_cost_minor: Option<i64>,
}

#[derive(sqlx::FromRow)]
struct IdempotencyRecord {
    response_body: String,
    request_hash: String,
}

// ============================================================================
// Service
// ============================================================================

/// Execute a valuation run.
///
/// Returns `(ValuationRunResult, is_replay)`:
/// - `is_replay = false`: run executed; HTTP 201.
/// - `is_replay = true`:  idempotency hit; HTTP 200 with stored result.
pub async fn execute_valuation_run(
    pool: &PgPool,
    req: &ValuationRunRequest,
) -> Result<(ValuationRunResult, bool), RunError> {
    validate_request(req)?;

    // --- Idempotency check ---
    let request_hash = serde_json::to_string(req)?;
    if let Some(record) = find_idempotency_key(pool, &req.tenant_id, &req.idempotency_key).await? {
        if record.request_hash != request_hash {
            return Err(RunError::ConflictingIdempotencyKey);
        }
        let result: ValuationRunResult = serde_json::from_str(&record.response_body)?;
        return Ok((result, true));
    }

    let created_at = Utc::now();
    let event_id = Uuid::new_v4();
    let run_id = Uuid::new_v4();
    let correlation_id = req
        .correlation_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let mut tx = pool.begin().await?;

    // --- Advisory lock: one valuation run per tenant at a time ---
    let lock_key = fnv_key(&format!("valrun:{}", req.tenant_id));
    let acquired: bool = sqlx::query_scalar("SELECT pg_try_advisory_xact_lock($1)")
        .bind(lock_key)
        .fetch_one(&mut *tx)
        .await?;
    if !acquired {
        return Err(RunError::ConcurrentRun);
    }

    // --- Query all layers for tenant/warehouse up to as_of ---
    let layer_rows: Vec<LayerRow> = sqlx::query_as::<_, LayerRow>(
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
    .bind(&req.tenant_id)
    .bind(req.warehouse_id)
    .bind(req.as_of)
    .fetch_all(&mut *tx)
    .await?;

    // --- Group layers by item ---
    let mut item_layers: BTreeMap<Uuid, Vec<FullLayer>> = BTreeMap::new();
    for row in &layer_rows {
        item_layers
            .entry(row.item_id)
            .or_default()
            .push(FullLayer {
                item_id: row.item_id,
                unit_cost_minor: row.unit_cost_minor,
                quantity_received: row.quantity_received,
                qty_consumed_at_as_of: row.qty_consumed_at_as_of,
            });
    }

    // --- Load standard costs if needed ---
    let standard_costs: BTreeMap<Uuid, i64> = if req.method == ValuationMethod::StandardCost {
        load_standard_costs(&mut tx, &req.tenant_id).await?
    } else {
        BTreeMap::new()
    };

    // --- Apply valuation method per item ---
    let mut lines: Vec<RunLineResult> = Vec::new();
    for (item_id, layers) in &item_layers {
        let valuation: Option<ItemValuation> = match req.method {
            ValuationMethod::Fifo => methods::value_fifo(layers),
            ValuationMethod::Lifo => methods::value_lifo(layers),
            ValuationMethod::Wac => methods::value_wac(layers),
            ValuationMethod::StandardCost => {
                if let Some(&std_cost) = standard_costs.get(item_id) {
                    methods::value_standard_cost(layers, std_cost)
                } else {
                    // No standard cost configured — skip this item
                    continue;
                }
            }
        };

        if let Some(v) = valuation {
            lines.push(RunLineResult {
                item_id: v.item_id,
                warehouse_id: req.warehouse_id,
                quantity_on_hand: v.quantity_on_hand,
                unit_cost_minor: v.unit_cost_minor,
                total_value_minor: v.total_value_minor,
                variance_minor: v.variance_minor,
                currency: req.currency.clone(),
            });
        }
    }

    let total_value_minor: i64 = lines.iter().map(|l| l.total_value_minor).sum();

    // --- Mutation: insert run header ---
    sqlx::query(
        r#"
        INSERT INTO valuation_runs
            (id, tenant_id, warehouse_id, method, as_of,
             total_value_minor, total_cogs_minor, currency)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        "#,
    )
    .bind(run_id)
    .bind(&req.tenant_id)
    .bind(req.warehouse_id)
    .bind(req.method.as_str())
    .bind(req.as_of)
    .bind(total_value_minor)
    .bind(0_i64)
    .bind(&req.currency)
    .execute(&mut *tx)
    .await?;

    // --- Mutation: batch insert per-item lines (single round-trip) ---
    if !lines.is_empty() {
        let item_ids: Vec<Uuid> = lines.iter().map(|l| l.item_id).collect();
        let warehouse_ids: Vec<Uuid> = lines.iter().map(|l| l.warehouse_id).collect();
        let qtys: Vec<i64> = lines.iter().map(|l| l.quantity_on_hand).collect();
        let unit_costs: Vec<i64> = lines.iter().map(|l| l.unit_cost_minor).collect();
        let total_values: Vec<i64> = lines.iter().map(|l| l.total_value_minor).collect();
        let variances: Vec<i64> = lines.iter().map(|l| l.variance_minor).collect();
        let currencies: Vec<&str> = vec![req.currency.as_str(); lines.len()];

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
        .bind(&item_ids)
        .bind(&warehouse_ids)
        .bind(&qtys)
        .bind(&unit_costs)
        .bind(&total_values)
        .bind(&variances)
        .bind(&currencies)
        .execute(&mut *tx)
        .await?;
    }

    // --- Outbox: emit inventory.valuation_run_completed ---
    let event_lines: Vec<ValuationRunCompletedLine> = lines
        .iter()
        .map(|l| ValuationRunCompletedLine {
            item_id: l.item_id,
            quantity_on_hand: l.quantity_on_hand,
            unit_cost_minor: l.unit_cost_minor,
            total_value_minor: l.total_value_minor,
            variance_minor: l.variance_minor,
        })
        .collect();

    let payload = ValuationRunCompletedPayload {
        run_id,
        tenant_id: req.tenant_id.clone(),
        warehouse_id: req.warehouse_id,
        method: req.method.as_str().to_string(),
        as_of: req.as_of,
        total_value_minor,
        total_cogs_minor: 0,
        currency: req.currency.clone(),
        line_count: lines.len(),
        lines: event_lines,
    };
    let envelope = build_valuation_run_completed_envelope(
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
            ($1, $2, 'valuation_run', $3, $4, $5::JSONB, $6, $7, '1.0.0')
        "#,
    )
    .bind(event_id)
    .bind(EVENT_TYPE_VALUATION_RUN_COMPLETED)
    .bind(run_id.to_string())
    .bind(&req.tenant_id)
    .bind(&envelope_json)
    .bind(&correlation_id)
    .bind(&req.causation_id)
    .execute(&mut *tx)
    .await?;

    // --- Idempotency: store response for replay ---
    let result = ValuationRunResult {
        run_id,
        tenant_id: req.tenant_id.clone(),
        warehouse_id: req.warehouse_id,
        method: req.method.as_str().to_string(),
        as_of: req.as_of,
        total_value_minor,
        total_cogs_minor: 0,
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
// Config management
// ============================================================================

/// Set the valuation method for an item. Upserts the configuration.
pub async fn set_item_valuation_method(
    pool: &PgPool,
    tenant_id: &str,
    item_id: Uuid,
    method: ValuationMethod,
    standard_cost_minor: Option<i64>,
) -> Result<(), RunError> {
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
    .bind(method.as_str())
    .bind(standard_cost_minor)
    .execute(pool)
    .await?;
    Ok(())
}

// ============================================================================
// Helpers
// ============================================================================

fn validate_request(req: &ValuationRunRequest) -> Result<(), RunError> {
    if req.tenant_id.trim().is_empty() {
        return Err(RunError::MissingTenant);
    }
    if req.idempotency_key.trim().is_empty() {
        return Err(RunError::MissingIdempotencyKey);
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

async fn load_standard_costs(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
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

    #[test]
    fn validate_rejects_empty_tenant() {
        let req = ValuationRunRequest {
            tenant_id: "".to_string(),
            warehouse_id: Uuid::new_v4(),
            method: ValuationMethod::Fifo,
            as_of: Utc::now(),
            idempotency_key: "k1".to_string(),
            currency: "usd".to_string(),
            correlation_id: None,
            causation_id: None,
        };
        assert!(matches!(validate_request(&req), Err(RunError::MissingTenant)));
    }

    #[test]
    fn validate_rejects_empty_idempotency_key() {
        let req = ValuationRunRequest {
            tenant_id: "t1".to_string(),
            warehouse_id: Uuid::new_v4(),
            method: ValuationMethod::Lifo,
            as_of: Utc::now(),
            idempotency_key: " ".to_string(),
            currency: "usd".to_string(),
            correlation_id: None,
            causation_id: None,
        };
        assert!(matches!(
            validate_request(&req),
            Err(RunError::MissingIdempotencyKey)
        ));
    }

    #[test]
    fn validate_accepts_valid_request() {
        let req = ValuationRunRequest {
            tenant_id: "t1".to_string(),
            warehouse_id: Uuid::new_v4(),
            method: ValuationMethod::Wac,
            as_of: Utc::now(),
            idempotency_key: "key-1".to_string(),
            currency: "usd".to_string(),
            correlation_id: None,
            causation_id: None,
        };
        assert!(validate_request(&req).is_ok());
    }

    #[test]
    fn fnv_key_is_deterministic() {
        assert_eq!(fnv_key("valrun:t1"), fnv_key("valrun:t1"));
        assert_ne!(fnv_key("valrun:t1"), fnv_key("valrun:t2"));
    }
}
