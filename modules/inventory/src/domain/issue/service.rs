//! Issue orchestration: Guard → Lock → FIFO → Mutation → Outbox.

use chrono::{Duration, Utc};
use event_bus::TracingContext;
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    domain::{
        fifo::{self, AvailableLayer},
        guards::{guard_convert_to_base, guard_item_active, guard_quantity_positive, GuardError},
        items::TrackingMode,
        lots_serials::issue as ls_issue,
        projections::on_hand,
        reorder::evaluator,
    },
    events::{
        contracts::{build_item_issued_envelope, ItemIssuedPayload, SourceRef},
        EVENT_TYPE_ITEM_ISSUED,
    },
};

use super::idempotency::{
    build_consumed_from_serials, check_idempotency, fetch_warehouse_totals, store_idempotency_key,
    write_layer_consumptions,
};
use super::types::{IssueError, IssueRequest, IssueResult, LayerRow};

/// Process a stock issue atomically.
///
/// Returns `(IssueResult, is_replay)`.
/// - `is_replay = false`: new issue created; HTTP 201.
/// - `is_replay = true`:  idempotency key matched; HTTP 200.
pub async fn process_issue(
    pool: &PgPool,
    req: &IssueRequest,
    tracing_ctx: Option<&TracingContext>,
) -> Result<(IssueResult, bool), IssueError> {
    // --- Stateless input validation ---
    validate_request(req)?;

    let request_hash = serde_json::to_string(req)?;

    // --- Idempotency fast-path ---
    if let Some(replay) = check_idempotency(pool, req, &request_hash).await? {
        return Ok(replay);
    }

    // --- Guard: item must exist and be active ---
    let item = guard_item_active(pool, req.item_id, &req.tenant_id).await?;

    // Tracking mode pre-checks (read-only)
    let lot_id: Option<Uuid> = match item.tracking_mode {
        TrackingMode::Lot => {
            let code = req.lot_code.as_deref().unwrap_or("").trim();
            if code.is_empty() {
                return Err(IssueError::LotRequired);
            }
            let id = ls_issue::find_lot_id(pool, &req.tenant_id, req.item_id, code)
                .await
                .map_err(IssueError::Database)?
                .ok_or_else(|| IssueError::LotNotFound(code.to_string()))?;
            Some(id)
        }
        TrackingMode::Serial => {
            let codes = req.serial_codes.as_deref().unwrap_or(&[]);
            if codes.is_empty() {
                return Err(IssueError::SerialRequired);
            }
            // Reject duplicate codes in the same request.
            let unique_count = codes.iter().collect::<std::collections::HashSet<_>>().len();
            if unique_count != codes.len() {
                return Err(IssueError::Guard(GuardError::Validation(
                    "serial_codes must be unique within a single issue request".to_string(),
                )));
            }
            None
        }
        TrackingMode::None => None,
    };

    // UoM conversion: serial-tracked uses serial_codes.len(); otherwise convert to base_uom.
    let quantity = match item.tracking_mode {
        TrackingMode::Serial => req.serial_codes.as_deref().unwrap_or(&[]).len() as i64,
        _ => {
            guard_convert_to_base(
                pool,
                req.item_id,
                &req.tenant_id,
                req.quantity,
                req.uom_id,
                item.base_uom_id,
            )
            .await?
        }
    };

    let event_id = Uuid::new_v4();
    let issued_at = Utc::now();
    let correlation_id = req
        .correlation_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let mut tx = pool.begin().await?;

    // --- Lock FIFO layers and compute consumed layers ---
    // Branching by tracking mode determines which layers are locked and how
    // the consumed slice is built.
    let (consumed, sum_remaining, pre_issue_total_cost, serial_ids_to_mark) = match item
        .tracking_mode
    {
        TrackingMode::Serial => {
            let codes = req.serial_codes.as_deref().unwrap_or(&[]);
            let locked =
                ls_issue::validate_and_lock_serials(&mut tx, &req.tenant_id, req.item_id, codes)
                    .await?;
            let serial_ids: Vec<Uuid> = locked.iter().map(|s| s.serial_id).collect();
            let consumed_layers = build_consumed_from_serials(&locked);

            let (wh_remaining, wh_cost) =
                fetch_warehouse_totals(&mut tx, &req.tenant_id, req.item_id, req.warehouse_id)
                    .await?;

            (consumed_layers, wh_remaining, wh_cost, serial_ids)
        }
        TrackingMode::Lot => {
            let lid = lot_id.expect("lot_id must be Some for Lot-tracked path");
            let layer_rows = sqlx::query_as::<_, LayerRow>(
                r#"
                    SELECT id, quantity_remaining, unit_cost_minor
                    FROM inventory_layers
                    WHERE tenant_id = $1
                      AND item_id   = $2
                      AND warehouse_id = $3
                      AND lot_id    = $4
                      AND quantity_remaining > 0
                    ORDER BY received_at ASC, ledger_entry_id ASC
                    FOR UPDATE
                    "#,
            )
            .bind(&req.tenant_id)
            .bind(req.item_id)
            .bind(req.warehouse_id)
            .bind(lid)
            .fetch_all(&mut *tx)
            .await?;

            if layer_rows.is_empty() {
                return Err(IssueError::NoLayersAvailable);
            }

            let available_layers: Vec<AvailableLayer> = layer_rows
                .iter()
                .map(|r| AvailableLayer {
                    layer_id: r.id,
                    quantity_remaining: r.quantity_remaining,
                    unit_cost_minor: r.unit_cost_minor,
                })
                .collect();

            let lot_sum: i64 = available_layers.iter().map(|l| l.quantity_remaining).sum();

            // Conservative availability: lot-layer sum minus warehouse reservations.
            let quantity_reserved: i64 = sqlx::query_scalar(
                r#"
                    SELECT COALESCE(quantity_reserved, 0)
                    FROM item_on_hand
                    WHERE tenant_id = $1 AND item_id = $2 AND warehouse_id = $3
                      AND location_id IS NULL
                    "#,
            )
            .bind(&req.tenant_id)
            .bind(req.item_id)
            .bind(req.warehouse_id)
            .fetch_optional(&mut *tx)
            .await?
            .unwrap_or(0i64);

            let net_available = lot_sum - quantity_reserved;
            if net_available < quantity {
                return Err(IssueError::InsufficientQuantity {
                    requested: quantity,
                    available: net_available,
                });
            }

            let consumed_layers = fifo::consume_fifo(&available_layers, quantity)?;

            let (wh_remaining, wh_cost) =
                fetch_warehouse_totals(&mut tx, &req.tenant_id, req.item_id, req.warehouse_id)
                    .await?;

            (consumed_layers, wh_remaining, wh_cost, vec![])
        }
        TrackingMode::None => {
            // Existing behavior: warehouse-wide FIFO.
            let layer_rows = sqlx::query_as::<_, LayerRow>(
                r#"
                    SELECT id, quantity_remaining, unit_cost_minor
                    FROM inventory_layers
                    WHERE tenant_id = $1
                      AND item_id   = $2
                      AND warehouse_id = $3
                      AND quantity_remaining > 0
                    ORDER BY received_at ASC, ledger_entry_id ASC
                    FOR UPDATE
                    "#,
            )
            .bind(&req.tenant_id)
            .bind(req.item_id)
            .bind(req.warehouse_id)
            .fetch_all(&mut *tx)
            .await?;

            let available_layers: Vec<AvailableLayer> = layer_rows
                .iter()
                .map(|r| AvailableLayer {
                    layer_id: r.id,
                    quantity_remaining: r.quantity_remaining,
                    unit_cost_minor: r.unit_cost_minor,
                })
                .collect();

            let sum_remaining: i64 = available_layers.iter().map(|l| l.quantity_remaining).sum();
            let pre_issue_total_cost: i64 = available_layers
                .iter()
                .map(|l| l.quantity_remaining * l.unit_cost_minor)
                .sum();

            // --- Availability check varies by whether a location is specified ---
            let net_available: i64 = if let Some(loc_id) = req.location_id {
                sqlx::query_scalar::<_, i64>(
                    r#"
                        SELECT COALESCE(available_status_on_hand, 0)
                        FROM item_on_hand
                        WHERE tenant_id    = $1
                          AND item_id      = $2
                          AND warehouse_id = $3
                          AND location_id  = $4
                        "#,
                )
                .bind(&req.tenant_id)
                .bind(req.item_id)
                .bind(req.warehouse_id)
                .bind(loc_id)
                .fetch_optional(&mut *tx)
                .await?
                .unwrap_or(0i64)
            } else {
                let quantity_reserved: i64 = sqlx::query_scalar(
                    r#"
                        SELECT COALESCE(quantity_reserved, 0)
                        FROM item_on_hand
                        WHERE tenant_id = $1 AND item_id = $2 AND warehouse_id = $3
                          AND location_id IS NULL
                        "#,
                )
                .bind(&req.tenant_id)
                .bind(req.item_id)
                .bind(req.warehouse_id)
                .fetch_optional(&mut *tx)
                .await?
                .unwrap_or(0i64);

                sum_remaining - quantity_reserved
            };

            if net_available < quantity {
                return Err(IssueError::InsufficientQuantity {
                    requested: quantity,
                    available: net_available,
                });
            }

            let consumed = fifo::consume_fifo(&available_layers, quantity)?;
            (consumed, sum_remaining, pre_issue_total_cost, vec![])
        }
    };

    let total_cost_minor: i64 = consumed.iter().map(|c| c.extended_cost_minor).sum();
    let post_issue_total_cost = (pre_issue_total_cost - total_cost_minor).max(0);
    let new_on_hand = sum_remaining - quantity;

    // --- Step 1: Insert ledger row (negative quantity = stock out) ---
    let ledger_row = sqlx::query_as::<_, super::types::LedgerRow>(
        r#"
        INSERT INTO inventory_ledger
            (tenant_id, item_id, warehouse_id, location_id, entry_type, quantity,
             unit_cost_minor, currency, source_event_id, source_event_type,
             reference_type, reference_id, posted_at)
        VALUES
            ($1, $2, $3, $4, 'issued', $5, 0, $6, $7, $8, $9, $10, $11)
        RETURNING id, entry_id
        "#,
    )
    .bind(&req.tenant_id)
    .bind(req.item_id)
    .bind(req.warehouse_id)
    .bind(req.location_id)
    .bind(-quantity) // signed: negative = stock out (base_uom units)
    .bind(&req.currency)
    .bind(event_id)
    .bind(EVENT_TYPE_ITEM_ISSUED)
    .bind(&req.source_type)
    .bind(&req.source_id)
    .bind(issued_at)
    .fetch_one(&mut *tx)
    .await?;

    let ledger_id = ledger_row.id;
    let issue_line_id = ledger_row.entry_id;

    // --- Step 2: Insert layer_consumptions + update layer quantity_remaining ---
    write_layer_consumptions(&mut tx, &consumed, ledger_id, issued_at)
        .await
        .map_err(IssueError::Database)?;

    // --- Step 2b: Mark serial instances as issued (after layer writes) ---
    if !serial_ids_to_mark.is_empty() {
        ls_issue::mark_serials_issued(&mut tx, &serial_ids_to_mark)
            .await
            .map_err(IssueError::Database)?;
    }

    // --- Step 3: Update on-hand projection ---
    if let Some(loc_id) = req.location_id {
        on_hand::decrement_for_issue(
            &mut tx,
            &req.tenant_id,
            req.item_id,
            req.warehouse_id,
            loc_id,
            quantity,
            total_cost_minor,
            ledger_id,
        )
        .await
        .map_err(IssueError::Database)?;

        on_hand::decrement_available_bucket(
            &mut tx,
            &req.tenant_id,
            req.item_id,
            req.warehouse_id,
            quantity,
        )
        .await
        .map_err(IssueError::Database)?;
    } else {
        on_hand::upsert_after_issue(
            &mut tx,
            &req.tenant_id,
            req.item_id,
            req.warehouse_id,
            new_on_hand,
            post_issue_total_cost,
            &req.currency,
            ledger_id,
        )
        .await
        .map_err(IssueError::Database)?;

        on_hand::set_available_bucket(
            &mut tx,
            &req.tenant_id,
            req.item_id,
            req.warehouse_id,
            new_on_hand,
        )
        .await
        .map_err(IssueError::Database)?;
    }

    // --- Step 4: Build and enqueue outbox event ---
    let source_ref = SourceRef {
        source_module: req.source_module.clone(),
        source_type: req.source_type.clone(),
        source_id: req.source_id.clone(),
        source_line_id: req.source_line_id.clone(),
    };

    let payload = ItemIssuedPayload {
        issue_line_id,
        tenant_id: req.tenant_id.clone(),
        item_id: req.item_id,
        sku: item.sku,
        warehouse_id: req.warehouse_id,
        quantity,
        total_cost_minor,
        currency: req.currency.clone(),
        consumed_layers: consumed.clone(),
        source_ref: source_ref.clone(),
        issued_at,
    };

    let default_ctx = TracingContext::default();
    let ctx = tracing_ctx.unwrap_or(&default_ctx);
    let envelope = build_item_issued_envelope(
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
    .bind(EVENT_TYPE_ITEM_ISSUED)
    .bind(req.item_id.to_string())
    .bind(&req.tenant_id)
    .bind(&envelope_json)
    .bind(&correlation_id)
    .bind(&req.causation_id)
    .execute(&mut *tx)
    .await?;

    // --- Step 5: Build result ---
    let result = IssueResult {
        issue_line_id,
        ledger_entry_id: ledger_id,
        event_id,
        tenant_id: req.tenant_id.clone(),
        item_id: req.item_id,
        warehouse_id: req.warehouse_id,
        location_id: req.location_id,
        quantity,
        total_cost_minor,
        currency: req.currency.clone(),
        consumed_layers: consumed,
        source_ref,
        issued_at,
    };

    // --- Step 6: Store idempotency key (expires in 7 days) ---
    let response_json = serde_json::to_string(&result)?;
    store_idempotency_key(
        &mut tx,
        &req.tenant_id,
        &req.idempotency_key,
        &request_hash,
        &response_json,
        201,
        issued_at + Duration::days(7),
    )
    .await?;

    tx.commit().await?;

    // Best-effort low-stock signal evaluation (errors do not fail the issue).
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

pub(super) fn validate_request(req: &IssueRequest) -> Result<(), IssueError> {
    if req.idempotency_key.trim().is_empty() {
        return Err(IssueError::Guard(GuardError::Validation(
            "idempotency_key is required".to_string(),
        )));
    }
    if req.tenant_id.trim().is_empty() {
        return Err(IssueError::Guard(GuardError::Validation(
            "tenant_id is required".to_string(),
        )));
    }
    if req.currency.trim().is_empty() {
        return Err(IssueError::Guard(GuardError::Validation(
            "currency is required".to_string(),
        )));
    }
    if req.source_module.trim().is_empty()
        || req.source_type.trim().is_empty()
        || req.source_id.trim().is_empty()
    {
        return Err(IssueError::Guard(GuardError::Validation(
            "source_module, source_type, and source_id are required".to_string(),
        )));
    }
    // Skip quantity check for serial-tracked items (quantity derived from serial_codes).
    if req.serial_codes.is_none() {
        guard_quantity_positive(req.quantity)?;
    }
    Ok(())
}
