//! Sales order service — business logic, state transitions, inventory integration.

use security::mint_service_jwt_with_context;
use sqlx::PgPool;
use uuid::Uuid;

use super::{
    compute_line_total, repo, CancelOrderRequest, CreateOrderLineRequest, CreateOrderRequest,
    ListOrdersQuery, OrderError, SalesOrder, SalesOrderLine, SalesOrderWithLines, SoStatus,
    UpdateOrderLineRequest, UpdateOrderRequest,
};

// ── Order CRUD ────────────────────────────────────────────────────────────────

pub async fn create_order(
    pool: &PgPool,
    tenant_id: &str,
    created_by: &str,
    req: CreateOrderRequest,
) -> Result<SalesOrder, OrderError> {
    use chrono::Utc;
    let id = Uuid::new_v4();
    let order_number = generate_order_number();
    let order_date = req
        .order_date
        .unwrap_or_else(|| chrono::Utc::now().date_naive());

    let order = repo::insert_order(
        pool,
        id,
        tenant_id,
        &order_number,
        req.customer_id,
        req.party_id,
        &req.currency,
        order_date,
        req.required_date,
        req.promised_date,
        req.external_quote_ref.as_deref(),
        None,
        None,
        req.notes.as_deref(),
        created_by,
    )
    .await?;
    let _ = Utc::now(); // suppress unused import warning
    Ok(order)
}

pub async fn get_order_with_lines(
    pool: &PgPool,
    tenant_id: &str,
    order_id: Uuid,
) -> Result<SalesOrderWithLines, OrderError> {
    let order = repo::fetch_order_for_mutation(pool, order_id, tenant_id)
        .await?
        .ok_or(OrderError::NotFound(order_id))?;
    let lines = repo::fetch_lines_for_order(pool, order_id, tenant_id).await?;
    Ok(SalesOrderWithLines { order, lines })
}

pub async fn list_orders(
    pool: &PgPool,
    tenant_id: &str,
    query: &ListOrdersQuery,
) -> Result<Vec<SalesOrder>, OrderError> {
    Ok(repo::list_orders(
        pool,
        tenant_id,
        query.customer_id,
        query.status.as_deref(),
        query.blanket_order_id,
        query.from_date,
        query.to_date,
        query.limit,
        query.offset,
    )
    .await?)
}

pub async fn update_order(
    pool: &PgPool,
    tenant_id: &str,
    order_id: Uuid,
    req: UpdateOrderRequest,
) -> Result<SalesOrder, OrderError> {
    let order = repo::fetch_order_for_mutation(pool, order_id, tenant_id)
        .await?
        .ok_or(OrderError::NotFound(order_id))?;

    let current_status = SoStatus::from_str(&order.status).unwrap_or(SoStatus::Draft);
    if current_status != SoStatus::Draft {
        return Err(OrderError::NotDraft(order.status.clone()));
    }

    let lines = repo::fetch_lines_for_order(pool, order_id, tenant_id).await?;
    let subtotal: i64 = lines.iter().map(|l| l.line_total_cents).sum();
    let tax = req.tax_cents.unwrap_or(order.tax_cents);
    let total = subtotal + tax;

    Ok(repo::update_order_header(
        pool,
        order_id,
        tenant_id,
        req.customer_id,
        req.party_id,
        req.required_date,
        req.promised_date,
        req.external_quote_ref.as_deref(),
        req.notes.as_deref(),
        tax,
        subtotal,
        total,
    )
    .await?)
}

// ── Line CRUD ─────────────────────────────────────────────────────────────────

pub async fn add_line(
    pool: &PgPool,
    tenant_id: &str,
    order_id: Uuid,
    req: CreateOrderLineRequest,
) -> Result<SalesOrderLine, OrderError> {
    let order = repo::fetch_order_for_mutation(pool, order_id, tenant_id)
        .await?
        .ok_or(OrderError::NotFound(order_id))?;

    if SoStatus::from_str(&order.status).unwrap_or(SoStatus::Draft) != SoStatus::Draft {
        return Err(OrderError::NotDraft(order.status));
    }

    let line_total = compute_line_total(req.quantity, req.unit_price_cents);
    let uom = req.uom.as_deref().unwrap_or("EA");
    let line_id = Uuid::new_v4();

    let line = repo::insert_line(
        pool,
        line_id,
        tenant_id,
        order_id,
        req.item_id,
        req.part_number.as_deref(),
        &req.description,
        uom,
        req.quantity,
        req.unit_price_cents,
        line_total,
        req.required_date,
        req.promised_date,
        req.warehouse_id,
        req.notes.as_deref(),
    )
    .await?;

    repo::recompute_and_save_totals(pool, order_id, tenant_id, order.tax_cents).await?;
    Ok(line)
}

pub async fn update_line(
    pool: &PgPool,
    tenant_id: &str,
    order_id: Uuid,
    line_id: Uuid,
    req: UpdateOrderLineRequest,
) -> Result<SalesOrderLine, OrderError> {
    let order = repo::fetch_order_for_mutation(pool, order_id, tenant_id)
        .await?
        .ok_or(OrderError::NotFound(order_id))?;

    if SoStatus::from_str(&order.status).unwrap_or(SoStatus::Draft) != SoStatus::Draft {
        return Err(OrderError::NotDraft(order.status));
    }

    let existing = repo::fetch_line(pool, line_id, tenant_id, order_id)
        .await?
        .ok_or(OrderError::NotFound(line_id))?;

    let quantity = req.quantity.unwrap_or(existing.quantity);
    let unit_price = req.unit_price_cents.unwrap_or(existing.unit_price_cents);
    let line_total = compute_line_total(quantity, unit_price);

    let line = repo::update_line(
        pool,
        line_id,
        tenant_id,
        order_id,
        req.item_id,
        req.part_number.as_deref(),
        req.description.as_deref(),
        req.uom.as_deref(),
        quantity,
        unit_price,
        line_total,
        req.required_date,
        req.promised_date,
        req.notes.as_deref(),
    )
    .await?;

    repo::recompute_and_save_totals(pool, order_id, tenant_id, order.tax_cents).await?;
    Ok(line)
}

pub async fn remove_line(
    pool: &PgPool,
    tenant_id: &str,
    order_id: Uuid,
    line_id: Uuid,
) -> Result<(), OrderError> {
    let order = repo::fetch_order_for_mutation(pool, order_id, tenant_id)
        .await?
        .ok_or(OrderError::NotFound(order_id))?;

    if SoStatus::from_str(&order.status).unwrap_or(SoStatus::Draft) != SoStatus::Draft {
        return Err(OrderError::NotDraft(order.status));
    }

    repo::delete_line(pool, line_id, tenant_id, order_id).await?;
    repo::recompute_and_save_totals(pool, order_id, tenant_id, order.tax_cents).await?;
    Ok(())
}

// ── State transitions ─────────────────────────────────────────────────────────

pub async fn book_order(
    pool: &PgPool,
    tenant_id: &str,
    order_id: Uuid,
    correlation_id: String,
    inventory_base_url: Option<&str>,
) -> Result<SalesOrder, OrderError> {
    let order = repo::fetch_order_for_mutation(pool, order_id, tenant_id)
        .await?
        .ok_or(OrderError::NotFound(order_id))?;

    let current = SoStatus::from_str(&order.status).unwrap_or(SoStatus::Draft);
    if !current.can_transition_to(SoStatus::Booked) {
        return Err(OrderError::InvalidTransition {
            from: order.status.clone(),
            to: "booked".to_string(),
        });
    }

    let lines = repo::fetch_lines_for_order(pool, order_id, tenant_id).await?;
    if lines.is_empty() {
        return Err(OrderError::EmptyLines);
    }

    // Transition to Booked first; inventory may further advance to InFulfillment
    repo::update_order_status(pool, order_id, tenant_id, SoStatus::Booked.as_str()).await?;

    // Sync inventory reservation for lines with item_id + warehouse_id
    if let Some(inv_url) = inventory_base_url {
        let client = reqwest::Client::new();
        let reservable: Vec<&SalesOrderLine> = lines
            .iter()
            .filter(|l| l.item_id.is_some() && l.warehouse_id.is_some())
            .collect();
        let reserved = reserve_lines_with_compensation(
            &client,
            inv_url,
            tenant_id,
            &reservable,
            &correlation_id,
        )
        .await?;

        // Persist reservation IDs
        for (line_id, reservation_id) in &reserved {
            repo::update_line_reservation(pool, *line_id, tenant_id, *reservation_id).await?;
        }

        // All reservable lines succeeded → advance to in_fulfillment
        repo::update_order_status(pool, order_id, tenant_id, SoStatus::InFulfillment.as_str())
            .await?;
    }

    repo::fetch_order_for_mutation(pool, order_id, tenant_id)
        .await?
        .ok_or(OrderError::NotFound(order_id))
}

pub async fn cancel_order(
    pool: &PgPool,
    tenant_id: &str,
    order_id: Uuid,
    _req: CancelOrderRequest,
    correlation_id: String,
    inventory_base_url: Option<&str>,
) -> Result<SalesOrder, OrderError> {
    let order = repo::fetch_order_for_mutation(pool, order_id, tenant_id)
        .await?
        .ok_or(OrderError::NotFound(order_id))?;

    let current = SoStatus::from_str(&order.status).unwrap_or(SoStatus::Draft);
    if !current.can_transition_to(SoStatus::Cancelled) {
        return Err(OrderError::InvalidTransition {
            from: order.status.clone(),
            to: "cancelled".to_string(),
        });
    }

    // Release any outstanding inventory reservations
    if let Some(inv_url) = inventory_base_url {
        let lines = repo::fetch_lines_for_order(pool, order_id, tenant_id).await?;
        let reserved: Vec<Uuid> = lines.iter().filter_map(|l| l.reservation_id).collect();
        if !reserved.is_empty() {
            let client = reqwest::Client::new();
            release_all_compensating(&client, inv_url, tenant_id, &reserved, &correlation_id).await;
        }
    }

    repo::update_order_status(pool, order_id, tenant_id, SoStatus::Cancelled.as_str()).await?;

    repo::fetch_order_for_mutation(pool, order_id, tenant_id)
        .await?
        .ok_or(OrderError::NotFound(order_id))
}

pub async fn transition_order(
    pool: &PgPool,
    tenant_id: &str,
    order_id: Uuid,
    to: SoStatus,
) -> Result<SalesOrder, OrderError> {
    let order = repo::fetch_order_for_mutation(pool, order_id, tenant_id)
        .await?
        .ok_or(OrderError::NotFound(order_id))?;

    let current = SoStatus::from_str(&order.status).unwrap_or(SoStatus::Draft);
    if !current.can_transition_to(to) {
        return Err(OrderError::InvalidTransition {
            from: order.status.clone(),
            to: to.as_str().to_string(),
        });
    }

    repo::update_order_status(pool, order_id, tenant_id, to.as_str()).await?;

    repo::fetch_order_for_mutation(pool, order_id, tenant_id)
        .await?
        .ok_or(OrderError::NotFound(order_id))
}

// ── Inventory integration ─────────────────────────────────────────────────────

fn service_jwt(tenant_id: &str) -> Option<String> {
    let tenant_uuid = tenant_id.parse::<Uuid>().unwrap_or(Uuid::nil());
    mint_service_jwt_with_context(tenant_uuid, Uuid::nil()).ok()
}

async fn reserve_lines_with_compensation(
    client: &reqwest::Client,
    inventory_base_url: &str,
    tenant_id: &str,
    lines: &[&SalesOrderLine],
    correlation_id: &str,
) -> Result<Vec<(Uuid, Uuid)>, OrderError> {
    let mut reserved: Vec<(Uuid, Uuid)> = Vec::new();
    let jwt = service_jwt(tenant_id);

    for line in lines {
        let item_id = match line.item_id {
            Some(id) => id,
            None => continue,
        };
        let warehouse_id = match line.warehouse_id {
            Some(id) => id,
            None => continue,
        };

        let idempotency_key = format!("{}-reserve-{}", correlation_id, line.id);
        let body = serde_json::json!({
            "item_id": item_id,
            "warehouse_id": warehouse_id,
            "quantity": line.quantity as i64,
            "reference_type": "sales_order",
            "reference_id": line.id.to_string(),
            "tenant_id": tenant_id,
            "idempotency_key": idempotency_key,
            "correlation_id": correlation_id,
        });

        let url = format!("{}/api/inventory/reservations/reserve", inventory_base_url);
        let mut req = client
            .post(&url)
            .header("x-tenant-id", tenant_id)
            .json(&body);
        if let Some(ref token) = jwt {
            req = req.header("Authorization", format!("Bearer {}", token));
        }
        let resp = req
            .send()
            .await
            .map_err(|e| OrderError::ReservationFailed {
                line_id: line.id,
                reason: e.to_string(),
            })?;

        if !resp.status().is_success() {
            let reason = resp.text().await.unwrap_or_else(|_| "unknown".to_string());
            // Compensate previously reserved lines before returning error
            let to_release: Vec<Uuid> = reserved.iter().map(|(_, r)| *r).collect();
            release_all_compensating(
                client,
                inventory_base_url,
                tenant_id,
                &to_release,
                correlation_id,
            )
            .await;
            return Err(OrderError::ReservationFailed {
                line_id: line.id,
                reason,
            });
        }

        let reservation_id: Uuid = resp
            .json::<serde_json::Value>()
            .await
            .ok()
            .and_then(|v| {
                v["reservation_id"]
                    .as_str()
                    .and_then(|s| s.parse().ok())
                    .or_else(|| v["id"].as_str().and_then(|s| s.parse().ok()))
            })
            .unwrap_or_else(Uuid::new_v4);

        reserved.push((line.id, reservation_id));
    }

    Ok(reserved)
}

/// Fire-and-log: releases all reservation IDs, never returns error, always logs failures.
async fn release_all_compensating(
    client: &reqwest::Client,
    inventory_base_url: &str,
    tenant_id: &str,
    reservation_ids: &[Uuid],
    correlation_id: &str,
) {
    let jwt = service_jwt(tenant_id);
    let url = format!("{}/api/inventory/reservations/release", inventory_base_url);

    for &reservation_id in reservation_ids {
        let idempotency_key = format!("{}-release-{}", correlation_id, reservation_id);
        let body = serde_json::json!({
            "reservation_id": reservation_id,
            "tenant_id": tenant_id,
            "idempotency_key": idempotency_key,
            "correlation_id": correlation_id,
        });
        let mut req = client
            .post(&url)
            .header("x-tenant-id", tenant_id)
            .json(&body);
        if let Some(ref token) = jwt {
            req = req.header("Authorization", format!("Bearer {}", token));
        }
        match req.send().await {
            Ok(r) if r.status().is_success() => {}
            Ok(r) => {
                tracing::error!(
                    correlation_id = %correlation_id,
                    reservation_id = %reservation_id,
                    status = %r.status(),
                    "Compensation release failed — operator must reconcile"
                );
            }
            Err(e) => {
                tracing::error!(
                    correlation_id = %correlation_id,
                    reservation_id = %reservation_id,
                    error = %e,
                    "Compensation release network error — operator must reconcile"
                );
            }
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn generate_order_number() -> String {
    use chrono::Utc;
    let now = Utc::now();
    format!(
        "SO-{}-{:06}",
        now.format("%Y%m%d"),
        fastrand::u32(0..1_000_000)
    )
}
