//! Stock adjustment service.
//!
//! Corrects physical reality without editing history.
//! Each adjustment creates:
//!   - A new inventory_ledger row (entry_type = 'adjusted', signed quantity)
//!   - A new inv_adjustments row (business key + reason)
//!   - An item_on_hand projection update (quantity_on_hand += delta)
//!   - An item_on_hand_by_status update (available bucket += delta)
//!   - An inventory.adjusted outbox event
//!
//! Guards:
//!   - Item must be active
//!   - quantity_delta != 0
//!   - reason must be non-empty
//!   - No-negative policy: negative delta requires on_hand >= abs(delta)
//!     unless allow_negative = true
//!
//! Pattern: Guard → Mutation → Outbox (all in one transaction)

use chrono::{Duration, Utc};
use event_bus::TracingContext;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::{
    domain::{guards::guard_item_active, reorder::evaluator},
    events::{build_adjusted_envelope, AdjustedPayload, EVENT_TYPE_ADJUSTED},
};

use super::types::{
    validate_request, AdjustError, AdjustRequest, AdjustResult, IdempotencyRecord, LedgerInserted,
    OnHandRow,
};

/// Process a stock adjustment atomically.
///
/// Returns `(AdjustResult, is_replay)`.
/// - `is_replay = false`: new adjustment created; HTTP 201.
/// - `is_replay = true`:  idempotency key matched; HTTP 200 with stored result.
pub async fn process_adjustment(
    pool: &PgPool,
    req: &AdjustRequest,
    tracing_ctx: Option<&TracingContext>,
) -> Result<(AdjustResult, bool), AdjustError> {
    // --- Stateless input validation ---
    validate_request(req)?;

    // --- Compute request hash for idempotency conflict detection ---
    let request_hash = serde_json::to_string(req)?;

    // --- Idempotency check (fast path for replays) ---
    if let Some(record) = find_idempotency_key(pool, &req.tenant_id, &req.idempotency_key).await? {
        if record.request_hash != request_hash {
            return Err(AdjustError::ConflictingIdempotencyKey);
        }
        let result: AdjustResult = serde_json::from_str(&record.response_body)?;
        return Ok((result, true));
    }

    // --- DB guard: item must exist and be active ---
    let item = guard_item_active(pool, req.item_id, &req.tenant_id).await?;

    // --- No-negative guard: check current on_hand for negative adjustments ---
    if req.quantity_delta < 0 && !req.allow_negative {
        let on_hand = get_on_hand(
            pool,
            &req.tenant_id,
            req.item_id,
            req.warehouse_id,
            req.location_id,
        )
        .await?;
        let would_be = on_hand + req.quantity_delta;
        if would_be < 0 {
            return Err(AdjustError::NegativeOnHand {
                available: on_hand,
                would_be,
            });
        }
    }

    let adjusted_at = Utc::now();
    let event_id = Uuid::new_v4();
    let correlation_id = req
        .correlation_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let mut tx = pool.begin().await?;

    // Step 1: Insert ledger row (entry_type = 'adjusted', zero cost in v1)
    let ledger = sqlx::query_as::<_, LedgerInserted>(
        r#"
        INSERT INTO inventory_ledger
            (tenant_id, item_id, warehouse_id, location_id, entry_type,
             quantity, unit_cost_minor, currency,
             source_event_id, source_event_type,
             reference_type, notes, posted_at)
        VALUES
            ($1, $2, $3, $4, 'adjusted', $5, 0, 'usd', $6, $7, 'adjustment', $8, $9)
        RETURNING id
        "#,
    )
    .bind(&req.tenant_id)
    .bind(req.item_id)
    .bind(req.warehouse_id)
    .bind(req.location_id)
    .bind(req.quantity_delta)
    .bind(event_id)
    .bind(EVENT_TYPE_ADJUSTED)
    .bind(&req.reason)
    .bind(adjusted_at)
    .fetch_one(&mut *tx)
    .await?;

    let ledger_entry_id = ledger.id;

    // Step 2: Insert inv_adjustments row (business key)
    let adjustment_id = sqlx::query_scalar::<_, Uuid>(
        r#"
        INSERT INTO inv_adjustments
            (tenant_id, item_id, warehouse_id, location_id,
             quantity_delta, reason, event_id, ledger_entry_id, adjusted_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        RETURNING id
        "#,
    )
    .bind(&req.tenant_id)
    .bind(req.item_id)
    .bind(req.warehouse_id)
    .bind(req.location_id)
    .bind(req.quantity_delta)
    .bind(&req.reason)
    .bind(event_id)
    .bind(ledger_entry_id)
    .bind(adjusted_at)
    .fetch_one(&mut *tx)
    .await?;

    // Step 3: Update item_on_hand projection
    upsert_on_hand(
        &mut tx,
        &req.tenant_id,
        req.item_id,
        req.warehouse_id,
        req.location_id,
        req.quantity_delta,
        ledger_entry_id,
    )
    .await?;

    // Step 4: Update item_on_hand_by_status available bucket
    upsert_available_bucket(
        &mut tx,
        &req.tenant_id,
        req.item_id,
        req.warehouse_id,
        req.quantity_delta,
    )
    .await?;

    // Step 5: Build outbox event
    let payload = AdjustedPayload {
        adjustment_id,
        tenant_id: req.tenant_id.clone(),
        item_id: req.item_id,
        sku: item.sku,
        warehouse_id: req.warehouse_id,
        quantity_delta: req.quantity_delta,
        reason: req.reason.clone(),
        adjusted_at,
    };
    let default_ctx = TracingContext::default();
    let ctx = tracing_ctx.unwrap_or(&default_ctx);
    let envelope = build_adjusted_envelope(
        event_id,
        req.tenant_id.clone(),
        correlation_id.clone(),
        req.causation_id.clone(),
        payload,
    )
    .with_tracing_context(ctx);
    let envelope_json = serde_json::to_string(&envelope)?;

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
    .bind(EVENT_TYPE_ADJUSTED)
    .bind(req.item_id.to_string())
    .bind(&req.tenant_id)
    .bind(&envelope_json)
    .bind(&correlation_id)
    .bind(&req.causation_id)
    .execute(&mut *tx)
    .await?;

    // Step 6: Store idempotency key (expires in 7 days)
    let result = AdjustResult {
        adjustment_id,
        ledger_entry_id,
        event_id,
        tenant_id: req.tenant_id.clone(),
        item_id: req.item_id,
        warehouse_id: req.warehouse_id,
        location_id: req.location_id,
        quantity_delta: req.quantity_delta,
        reason: req.reason.clone(),
        adjusted_at,
    };
    let response_json = serde_json::to_string(&result)?;
    let expires_at = adjusted_at + Duration::days(7);

    sqlx::query(
        r#"
        INSERT INTO inv_idempotency_keys
            (tenant_id, idempotency_key, request_hash, response_body, status_code, expires_at)
        VALUES
            ($1, $2, $3, $4::JSONB, 201, $5)
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

    // Best-effort low-stock signal evaluation (errors do not fail the adjustment).
    let _ = evaluator::evaluate_low_stock(
        pool,
        &req.tenant_id,
        req.item_id,
        req.warehouse_id,
        req.location_id,
        &correlation_id,
        req.causation_id.clone(),
    )
    .await;

    Ok((result, false))
}

// ============================================================================
// Helpers
// ============================================================================

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

/// Read current on_hand quantity (0 if no row exists).
async fn get_on_hand(
    pool: &PgPool,
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
    location_id: Option<Uuid>,
) -> Result<i64, sqlx::Error> {
    let row = match location_id {
        None => {
            sqlx::query_as::<_, OnHandRow>(
                r#"
                SELECT quantity_on_hand
                FROM item_on_hand
                WHERE tenant_id = $1 AND item_id = $2 AND warehouse_id = $3
                  AND location_id IS NULL
                "#,
            )
            .bind(tenant_id)
            .bind(item_id)
            .bind(warehouse_id)
            .fetch_optional(pool)
            .await?
        }
        Some(loc_id) => {
            sqlx::query_as::<_, OnHandRow>(
                r#"
                SELECT quantity_on_hand
                FROM item_on_hand
                WHERE tenant_id = $1 AND item_id = $2 AND warehouse_id = $3
                  AND location_id = $4
                "#,
            )
            .bind(tenant_id)
            .bind(item_id)
            .bind(warehouse_id)
            .bind(loc_id)
            .fetch_optional(pool)
            .await?
        }
    };
    Ok(row.map(|r| r.quantity_on_hand).unwrap_or(0))
}

/// Upsert item_on_hand projection after an adjustment.
async fn upsert_on_hand(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
    location_id: Option<Uuid>,
    delta: i64,
    ledger_entry_id: i64,
) -> Result<(), sqlx::Error> {
    match location_id {
        None => {
            sqlx::query(
                r#"
                INSERT INTO item_on_hand
                    (tenant_id, item_id, warehouse_id, location_id,
                     quantity_on_hand, available_status_on_hand,
                     total_cost_minor, currency, last_ledger_entry_id, projected_at)
                VALUES ($1, $2, $3, NULL, $4, $4, 0, 'usd', $5, NOW())
                ON CONFLICT (tenant_id, item_id, warehouse_id)
                WHERE location_id IS NULL
                DO UPDATE
                    SET quantity_on_hand         = item_on_hand.quantity_on_hand + $4,
                        available_status_on_hand = item_on_hand.available_status_on_hand + $4,
                        last_ledger_entry_id     = $5,
                        projected_at             = NOW()
                "#,
            )
            .bind(tenant_id)
            .bind(item_id)
            .bind(warehouse_id)
            .bind(delta)
            .bind(ledger_entry_id)
            .execute(&mut **tx)
            .await?;
        }
        Some(loc_id) => {
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
            .bind(loc_id)
            .bind(delta)
            .bind(ledger_entry_id)
            .execute(&mut **tx)
            .await?;
        }
    }
    Ok(())
}

/// Update the 'available' status bucket after an adjustment.
///
/// v1: adjustments always affect the 'available' bucket.
///
/// Positive delta: upsert (create row if not exists, otherwise increment).
///
/// Negative delta: UPDATE only — PostgreSQL evaluates CHECK constraints during
/// the INSERT phase of INSERT...ON CONFLICT, before conflict detection.  Inserting
/// a negative value would violate `quantity_on_hand >= 0` even when an existing
/// row would have made the ON CONFLICT UPDATE path yield a valid result.
/// `GREATEST(0, ...)` handles the allow_negative edge case where the delta
/// exceeds the bucket (bucket floors at 0; item_on_hand carries the true signed value).
async fn upsert_available_bucket(
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
