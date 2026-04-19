// Outbox events are enqueued via `repo::enqueue_outbox` (no separate outbox/ module — pattern is inline here).
use sqlx::PgPool;
use uuid::Uuid;

use super::models::*;
use super::state_machine;

// ── OP Orders ─────────────────────────────────────────────────────────────────

pub async fn create_order(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &str,
    req: &CreateOpOrderRequest,
    op_order_number: &str,
) -> Result<OpOrder, OpError> {
    let order = sqlx::query_as::<_, OpOrder>(
        r#"
        INSERT INTO op_orders (
            tenant_id, op_order_number, status, vendor_id, service_type,
            service_description, process_spec_ref, part_number, part_revision,
            quantity_sent, unit_of_measure, work_order_id, operation_id,
            lot_id, serial_numbers, expected_ship_date, expected_return_date,
            estimated_cost_cents, notes, created_by
        ) VALUES (
            $1, $2, 'draft', $3, $4, $5, $6, $7, $8,
            $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19
        )
        RETURNING *
        "#,
    )
    .bind(tenant_id)
    .bind(op_order_number)
    .bind(req.vendor_id)
    .bind(&req.service_type)
    .bind(&req.service_description)
    .bind(&req.process_spec_ref)
    .bind(&req.part_number)
    .bind(&req.part_revision)
    .bind(req.quantity_sent)
    .bind(req.unit_of_measure.as_deref().unwrap_or("ea"))
    .bind(req.work_order_id)
    .bind(req.operation_id)
    .bind(req.lot_id)
    .bind(req.serial_numbers.as_deref().unwrap_or(&[]))
    .bind(req.expected_ship_date)
    .bind(req.expected_return_date)
    .bind(req.estimated_cost_cents)
    .bind(&req.notes)
    .bind(&req.created_by)
    .fetch_one(&mut **tx)
    .await?;

    Ok(order)
}

pub async fn get_order(
    pool: &PgPool,
    tenant_id: &str,
    order_id: Uuid,
) -> Result<Option<OpOrder>, OpError> {
    let order = sqlx::query_as::<_, OpOrder>(
        "SELECT * FROM op_orders WHERE op_order_id = $1 AND tenant_id = $2",
    )
    .bind(order_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;

    Ok(order)
}

pub async fn get_order_detail(
    pool: &PgPool,
    tenant_id: &str,
    order_id: Uuid,
) -> Result<Option<OpOrderDetail>, OpError> {
    let order = match get_order(pool, tenant_id, order_id).await? {
        Some(o) => o,
        None => return Ok(None),
    };

    let ship_events = list_ship_events(pool, tenant_id, order_id).await?;
    let return_events = list_return_events(pool, tenant_id, order_id).await?;
    let reviews = list_reviews(pool, tenant_id, order_id).await?;
    let re_identifications = list_re_identifications(pool, tenant_id, order_id).await?;

    Ok(Some(OpOrderDetail {
        order,
        ship_events,
        return_events,
        reviews,
        re_identifications,
    }))
}

pub async fn list_orders(
    pool: &PgPool,
    tenant_id: &str,
    q: &ListOpOrdersQuery,
) -> Result<Vec<OpOrder>, OpError> {
    let limit = q.limit.unwrap_or(50).min(200);
    let offset = q.offset.unwrap_or(0);

    let orders = sqlx::query_as::<_, OpOrder>(
        r#"
        SELECT * FROM op_orders
        WHERE tenant_id = $1
          AND ($2::TEXT IS NULL OR status = $2)
          AND ($3::UUID IS NULL OR vendor_id = $3)
          AND ($4::UUID IS NULL OR work_order_id = $4)
          AND ($5::TEXT IS NULL OR service_type = $5)
          AND ($6::DATE IS NULL OR created_at::DATE >= $6)
          AND ($7::DATE IS NULL OR created_at::DATE <= $7)
        ORDER BY created_at DESC
        LIMIT $8 OFFSET $9
        "#,
    )
    .bind(tenant_id)
    .bind(&q.status)
    .bind(q.vendor_id)
    .bind(q.work_order_id)
    .bind(&q.service_type)
    .bind(q.from_date)
    .bind(q.to_date)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    Ok(orders)
}

pub async fn update_order(
    pool: &PgPool,
    tenant_id: &str,
    order_id: Uuid,
    req: &UpdateOpOrderRequest,
) -> Result<OpOrder, OpError> {
    let order = sqlx::query_as::<_, OpOrder>(
        r#"
        UPDATE op_orders SET
            vendor_id = COALESCE($3, vendor_id),
            service_type = COALESCE($4, service_type),
            service_description = COALESCE($5, service_description),
            process_spec_ref = COALESCE($6, process_spec_ref),
            part_number = COALESCE($7, part_number),
            part_revision = COALESCE($8, part_revision),
            quantity_sent = COALESCE($9, quantity_sent),
            unit_of_measure = COALESCE($10, unit_of_measure),
            work_order_id = COALESCE($11, work_order_id),
            operation_id = COALESCE($12, operation_id),
            lot_id = COALESCE($13, lot_id),
            serial_numbers = COALESCE($14, serial_numbers),
            expected_ship_date = COALESCE($15, expected_ship_date),
            expected_return_date = COALESCE($16, expected_return_date),
            estimated_cost_cents = COALESCE($17, estimated_cost_cents),
            notes = COALESCE($18, notes),
            updated_at = now()
        WHERE op_order_id = $1 AND tenant_id = $2
          AND status IN ('draft', 'issued')
        RETURNING *
        "#,
    )
    .bind(order_id)
    .bind(tenant_id)
    .bind(req.vendor_id)
    .bind(&req.service_type)
    .bind(&req.service_description)
    .bind(&req.process_spec_ref)
    .bind(&req.part_number)
    .bind(&req.part_revision)
    .bind(req.quantity_sent)
    .bind(&req.unit_of_measure)
    .bind(req.work_order_id)
    .bind(req.operation_id)
    .bind(req.lot_id)
    .bind(req.serial_numbers.as_deref())
    .bind(req.expected_ship_date)
    .bind(req.expected_return_date)
    .bind(req.estimated_cost_cents)
    .bind(&req.notes)
    .fetch_optional(pool)
    .await?;

    order.ok_or(OpError::NotFound(order_id))
}

pub async fn set_order_status(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &str,
    order_id: Uuid,
    new_status: &str,
) -> Result<OpOrder, OpError> {
    let order = sqlx::query_as::<_, OpOrder>(
        r#"
        UPDATE op_orders SET status = $3, updated_at = now()
        WHERE op_order_id = $1 AND tenant_id = $2
        RETURNING *
        "#,
    )
    .bind(order_id)
    .bind(tenant_id)
    .bind(new_status)
    .fetch_optional(&mut **tx)
    .await?;

    order.ok_or(OpError::NotFound(order_id))
}

/// Lock the order row for update within a transaction (no quantity aggregates).
pub async fn lock_order(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &str,
    order_id: Uuid,
) -> Result<OpOrder, OpError> {
    let order = sqlx::query_as::<_, OpOrder>(
        "SELECT * FROM op_orders WHERE op_order_id = $1 AND tenant_id = $2 FOR UPDATE",
    )
    .bind(order_id)
    .bind(tenant_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(OpError::NotFound(order_id))?;

    Ok(order)
}

/// Lock order row for quantity-bound checks. Returns (order, sum_shipped, sum_received).
pub async fn lock_order_for_quantity_check(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &str,
    order_id: Uuid,
) -> Result<(OpOrder, i64, i64), OpError> {
    let order = sqlx::query_as::<_, OpOrder>(
        "SELECT * FROM op_orders WHERE op_order_id = $1 AND tenant_id = $2 FOR UPDATE",
    )
    .bind(order_id)
    .bind(tenant_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(OpError::NotFound(order_id))?;

    let sum_shipped: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(quantity_shipped), 0) FROM op_ship_events WHERE op_order_id = $1",
    )
    .bind(order_id)
    .fetch_one(&mut **tx)
    .await?;

    let sum_received: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(quantity_received), 0) FROM op_return_events WHERE op_order_id = $1",
    )
    .bind(order_id)
    .fetch_one(&mut **tx)
    .await?;

    Ok((order, sum_shipped, sum_received))
}

// ── Issue ────────────────────────────────────────────────────────────────────

pub async fn issue_order(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &str,
    order_id: Uuid,
    purchase_order_id: Option<Uuid>,
) -> Result<OpOrder, OpError> {
    let order = sqlx::query_as::<_, OpOrder>(
        "SELECT * FROM op_orders WHERE op_order_id = $1 AND tenant_id = $2 FOR UPDATE",
    )
    .bind(order_id)
    .bind(tenant_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(OpError::NotFound(order_id))?;

    let _ = state_machine::transition_issue(&order.status)?;

    if order.vendor_id.is_none() {
        return Err(OpError::Validation(
            "vendor_id is required before issuing".to_string(),
        ));
    }
    if order.service_type.is_none() {
        return Err(OpError::Validation(
            "service_type is required before issuing".to_string(),
        ));
    }
    if order.quantity_sent <= 0 {
        return Err(OpError::Validation(
            "quantity_sent must be > 0 before issuing".to_string(),
        ));
    }

    let updated = sqlx::query_as::<_, OpOrder>(
        r#"
        UPDATE op_orders SET
            status = 'issued',
            purchase_order_id = COALESCE($3, purchase_order_id),
            updated_at = now()
        WHERE op_order_id = $1 AND tenant_id = $2
        RETURNING *
        "#,
    )
    .bind(order_id)
    .bind(tenant_id)
    .bind(purchase_order_id)
    .fetch_one(&mut **tx)
    .await?;

    Ok(updated)
}

// ── Ship Events ────────────────────────────────────────────────────────────────

pub async fn create_ship_event_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &str,
    order_id: Uuid,
    req: &CreateShipEventRequest,
) -> Result<OpShipEvent, OpError> {
    let event = sqlx::query_as::<_, OpShipEvent>(
        r#"
        INSERT INTO op_ship_events (
            tenant_id, op_order_id, ship_date, quantity_shipped, unit_of_measure,
            lot_number, serial_numbers, carrier_name, tracking_number,
            packing_slip_number, shipped_by, shipping_reference, notes
        ) VALUES (
            $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13
        )
        RETURNING *
        "#,
    )
    .bind(tenant_id)
    .bind(order_id)
    .bind(req.ship_date)
    .bind(req.quantity_shipped)
    .bind(req.unit_of_measure.as_deref().unwrap_or("ea"))
    .bind(&req.lot_number)
    .bind(req.serial_numbers.as_deref().unwrap_or(&[]))
    .bind(&req.carrier_name)
    .bind(&req.tracking_number)
    .bind(&req.packing_slip_number)
    .bind(&req.shipped_by)
    .bind(req.shipping_reference)
    .bind(&req.notes)
    .fetch_one(&mut **tx)
    .await?;

    Ok(event)
}

pub async fn list_ship_events(
    pool: &PgPool,
    tenant_id: &str,
    order_id: Uuid,
) -> Result<Vec<OpShipEvent>, OpError> {
    let events = sqlx::query_as::<_, OpShipEvent>(
        "SELECT * FROM op_ship_events WHERE op_order_id = $1 AND tenant_id = $2 ORDER BY created_at ASC",
    )
    .bind(order_id)
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;

    Ok(events)
}

// ── Return Events ──────────────────────────────────────────────────────────────

pub async fn create_return_event_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &str,
    order_id: Uuid,
    req: &CreateReturnEventRequest,
) -> Result<OpReturnEvent, OpError> {
    let event = sqlx::query_as::<_, OpReturnEvent>(
        r#"
        INSERT INTO op_return_events (
            tenant_id, op_order_id, received_date, quantity_received, unit_of_measure,
            condition, discrepancy_notes, lot_number, serial_numbers, cert_ref,
            vendor_packing_slip, carrier_name, tracking_number, re_identification_required,
            received_by, notes
        ) VALUES (
            $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16
        )
        RETURNING *
        "#,
    )
    .bind(tenant_id)
    .bind(order_id)
    .bind(req.received_date)
    .bind(req.quantity_received)
    .bind(req.unit_of_measure.as_deref().unwrap_or("ea"))
    .bind(req.condition.as_str())
    .bind(&req.discrepancy_notes)
    .bind(&req.lot_number)
    .bind(req.serial_numbers.as_deref().unwrap_or(&[]))
    .bind(&req.cert_ref)
    .bind(&req.vendor_packing_slip)
    .bind(&req.carrier_name)
    .bind(&req.tracking_number)
    .bind(req.re_identification_required.unwrap_or(false))
    .bind(&req.received_by)
    .bind(&req.notes)
    .fetch_one(&mut **tx)
    .await?;

    Ok(event)
}

pub async fn list_return_events(
    pool: &PgPool,
    tenant_id: &str,
    order_id: Uuid,
) -> Result<Vec<OpReturnEvent>, OpError> {
    let events = sqlx::query_as::<_, OpReturnEvent>(
        "SELECT * FROM op_return_events WHERE op_order_id = $1 AND tenant_id = $2 ORDER BY created_at ASC",
    )
    .bind(order_id)
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;

    Ok(events)
}

pub async fn count_return_events(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    order_id: Uuid,
) -> Result<i64, OpError> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM op_return_events WHERE op_order_id = $1",
    )
    .bind(order_id)
    .fetch_one(&mut **tx)
    .await?;

    Ok(count)
}

pub async fn return_event_exists(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &str,
    order_id: Uuid,
    return_event_id: Uuid,
) -> Result<bool, OpError> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM op_return_events WHERE id = $1 AND op_order_id = $2 AND tenant_id = $3)",
    )
    .bind(return_event_id)
    .bind(order_id)
    .bind(tenant_id)
    .fetch_one(&mut **tx)
    .await?;

    Ok(exists)
}

// ── Vendor Reviews ────────────────────────────────────────────────────────────

pub async fn create_review_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &str,
    order_id: Uuid,
    req: &CreateReviewRequest,
) -> Result<OpVendorReview, OpError> {
    let review = sqlx::query_as::<_, OpVendorReview>(
        r#"
        INSERT INTO op_vendor_reviews (
            tenant_id, op_order_id, return_event_id, outcome, conditions,
            rejection_reason, reviewed_by, reviewed_at, notes
        ) VALUES (
            $1, $2, $3, $4, $5, $6, $7, $8, $9
        )
        RETURNING *
        "#,
    )
    .bind(tenant_id)
    .bind(order_id)
    .bind(req.return_event_id)
    .bind(req.outcome.as_str())
    .bind(&req.conditions)
    .bind(&req.rejection_reason)
    .bind(&req.reviewed_by)
    .bind(req.reviewed_at)
    .bind(&req.notes)
    .fetch_one(&mut **tx)
    .await?;

    Ok(review)
}

pub async fn list_reviews(
    pool: &PgPool,
    tenant_id: &str,
    order_id: Uuid,
) -> Result<Vec<OpVendorReview>, OpError> {
    let reviews = sqlx::query_as::<_, OpVendorReview>(
        "SELECT * FROM op_vendor_reviews WHERE op_order_id = $1 AND tenant_id = $2 ORDER BY created_at ASC",
    )
    .bind(order_id)
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;

    Ok(reviews)
}

// ── Re-Identifications ────────────────────────────────────────────────────────

pub async fn create_re_identification_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &str,
    order_id: Uuid,
    req: &CreateReIdentificationRequest,
) -> Result<OpReIdentification, OpError> {
    let reid = sqlx::query_as::<_, OpReIdentification>(
        r#"
        INSERT INTO op_re_identifications (
            tenant_id, op_order_id, return_event_id, old_part_number, old_part_revision,
            new_part_number, new_part_revision, reason, performed_by, performed_at
        ) VALUES (
            $1, $2, $3, $4, $5, $6, $7, $8, $9, $10
        )
        RETURNING *
        "#,
    )
    .bind(tenant_id)
    .bind(order_id)
    .bind(req.return_event_id)
    .bind(&req.old_part_number)
    .bind(&req.old_part_revision)
    .bind(&req.new_part_number)
    .bind(&req.new_part_revision)
    .bind(&req.reason)
    .bind(&req.performed_by)
    .bind(req.performed_at)
    .fetch_one(&mut **tx)
    .await?;

    Ok(reid)
}

pub async fn list_re_identifications(
    pool: &PgPool,
    tenant_id: &str,
    order_id: Uuid,
) -> Result<Vec<OpReIdentification>, OpError> {
    let reids = sqlx::query_as::<_, OpReIdentification>(
        "SELECT * FROM op_re_identifications WHERE op_order_id = $1 AND tenant_id = $2 ORDER BY created_at ASC",
    )
    .bind(order_id)
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;

    Ok(reids)
}

// ── Labels ────────────────────────────────────────────────────────────────────

pub async fn list_status_labels(
    pool: &PgPool,
    tenant_id: &str,
) -> Result<Vec<OpStatusLabel>, OpError> {
    let labels = sqlx::query_as::<_, OpStatusLabel>(
        "SELECT * FROM op_status_labels WHERE tenant_id = $1 ORDER BY canonical_status",
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;

    Ok(labels)
}

pub async fn upsert_status_label(
    pool: &PgPool,
    tenant_id: &str,
    canonical_status: &str,
    req: &UpsertStatusLabelRequest,
) -> Result<OpStatusLabel, OpError> {
    let label = sqlx::query_as::<_, OpStatusLabel>(
        r#"
        INSERT INTO op_status_labels (tenant_id, canonical_status, display_label, description, updated_by)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (tenant_id, canonical_status)
        DO UPDATE SET
            display_label = EXCLUDED.display_label,
            description = EXCLUDED.description,
            updated_by = EXCLUDED.updated_by,
            updated_at = now()
        RETURNING *
        "#,
    )
    .bind(tenant_id)
    .bind(canonical_status)
    .bind(&req.display_label)
    .bind(&req.description)
    .bind(&req.updated_by)
    .fetch_one(pool)
    .await?;

    Ok(label)
}

pub async fn list_service_type_labels(
    pool: &PgPool,
    tenant_id: &str,
) -> Result<Vec<OpServiceTypeLabel>, OpError> {
    let labels = sqlx::query_as::<_, OpServiceTypeLabel>(
        "SELECT * FROM op_service_type_labels WHERE tenant_id = $1 ORDER BY service_type",
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;

    Ok(labels)
}

pub async fn upsert_service_type_label(
    pool: &PgPool,
    tenant_id: &str,
    service_type: &str,
    req: &UpsertServiceTypeLabelRequest,
) -> Result<OpServiceTypeLabel, OpError> {
    let label = sqlx::query_as::<_, OpServiceTypeLabel>(
        r#"
        INSERT INTO op_service_type_labels (tenant_id, service_type, display_label, description, updated_by)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (tenant_id, service_type)
        DO UPDATE SET
            display_label = EXCLUDED.display_label,
            description = EXCLUDED.description,
            updated_by = EXCLUDED.updated_by,
            updated_at = now()
        RETURNING *
        "#,
    )
    .bind(tenant_id)
    .bind(service_type)
    .bind(&req.display_label)
    .bind(&req.description)
    .bind(&req.updated_by)
    .fetch_one(pool)
    .await?;

    Ok(label)
}

// ── Outbox helper ─────────────────────────────────────────────────────────────

pub async fn enqueue_outbox<T: serde::Serialize>(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &str,
    event_id: Uuid,
    event_type: &str,
    aggregate_type: &str,
    aggregate_id: &str,
    envelope: &event_bus::EventEnvelope<T>,
    correlation_id: &str,
    causation_id: Option<&str>,
) -> Result<(), OpError> {
    let payload = serde_json::to_value(envelope)
        .map_err(|e| OpError::Validation(format!("event serialization: {}", e)))?;

    sqlx::query(
        r#"
        INSERT INTO op_outbox
            (event_id, event_type, aggregate_type, aggregate_id, tenant_id,
             payload, correlation_id, causation_id, schema_version)
        VALUES ($1, $2, $3, $4, $5, $6::JSONB, $7, $8, '1.0.0')
        ON CONFLICT (event_id) DO NOTHING
        "#,
    )
    .bind(event_id)
    .bind(event_type)
    .bind(aggregate_type)
    .bind(aggregate_id)
    .bind(tenant_id)
    .bind(payload)
    .bind(correlation_id)
    .bind(causation_id)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

/// Generate a sequential OP order number (advisory-lock-safe for concurrency).
pub async fn next_op_order_number(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &str,
) -> Result<String, OpError> {
    // Acquire a session-level transaction advisory lock keyed on tenant to prevent
    // concurrent transactions from generating duplicate order numbers.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
        .bind(tenant_id)
        .execute(&mut **tx)
        .await?;

    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM op_orders WHERE tenant_id = $1",
    )
    .bind(tenant_id)
    .fetch_one(&mut **tx)
    .await?;

    Ok(format!("OP-{:06}", count + 1))
}

/// Check idempotency key — returns true if this event was already processed.
pub async fn is_event_processed(
    pool: &PgPool,
    event_id: Uuid,
    processor: &str,
) -> Result<bool, sqlx::Error> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM op_processed_events WHERE event_id = $1 AND processor = $2)",
    )
    .bind(event_id)
    .bind(processor)
    .fetch_one(pool)
    .await?;

    Ok(exists)
}

pub async fn mark_event_processed(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    event_id: Uuid,
    event_type: &str,
    processor: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO op_processed_events (event_id, event_type, processor)
        VALUES ($1, $2, $3)
        ON CONFLICT (event_id) DO NOTHING
        "#,
    )
    .bind(event_id)
    .bind(event_type)
    .bind(processor)
    .execute(&mut **tx)
    .await?;

    Ok(())
}
