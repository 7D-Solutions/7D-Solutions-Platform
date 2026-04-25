//! AR consumer for shipping_receiving.shipping_cost.incurred events.
//!
//! When a shipping label is created and the carrier charged a customer-facing
//! amount (customer_charge_minor is Some), this consumer:
//!   1. Finds the AR invoice referenced by order_ref.
//!   2. If the invoice is mutable (draft or open), appends a shipping line and
//!      updates amount_cents.
//!   3. If the invoice is immutable (paid, void, uncollectible), emits a
//!      `ar.customer_charge_after_post` warning event so finance can issue a
//!      supplementary invoice. The original event is never silently dropped.
//!
//! ## Idempotency
//! Uses the AR processed_events table. A duplicate delivery is a no-op.
//!
//! ## Skipped cases
//! - customer_charge_minor is None → no customer charge, skip.
//! - order_ref is None → no invoice reference, skip.
//! - Invoice not found for (order_ref, tenant) → log warning, skip.

use chrono::Utc;
use event_bus::{BusMessage, EventBus, EventEnvelope};
use futures::StreamExt;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

use crate::events::{is_event_processed, mark_event_processed};

/// Subject for the warning event emitted when we can't add to a posted invoice.
pub const EVENT_TYPE_CUSTOMER_CHARGE_AFTER_POST: &str = "ar.customer_charge_after_post";

// ============================================================================
// Local payload mirror (anti-corruption layer)
// mirrors shipping_receiving::events::contracts::shipping_cost::ShippingCostIncurredPayload
// ============================================================================

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ShippingCostIncurredPayload {
    pub tenant_id: String,
    pub shipment_id: Uuid,
    pub tracking_number: String,
    pub carrier_code: String,
    pub customer_charge_minor: Option<i64>,
    pub currency: String,
    pub order_ref: Option<String>,
    pub incurred_at: chrono::DateTime<Utc>,
}

// ============================================================================
// Public processing function (testable without NATS)
// ============================================================================

/// Process a single shipping_cost.incurred event for the AR side.
///
/// Returns Ok in all cases where the event is handled gracefully (including
/// skips). Returns Err only on unexpected database errors.
pub async fn handle_shipping_cost_incurred(
    pool: &PgPool,
    event_id: Uuid,
    payload: &ShippingCostIncurredPayload,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Idempotency check — already processed events are no-ops.
    if is_event_processed(pool, event_id).await? {
        tracing::debug!(
            event_id = %event_id,
            "AR: duplicate shipping_cost.incurred event ignored"
        );
        return Ok(());
    }

    // Skip if there is no customer charge.
    let customer_charge = match payload.customer_charge_minor {
        Some(c) if c > 0 => c,
        _ => {
            tracing::debug!(
                event_id = %event_id,
                tracking_number = %payload.tracking_number,
                "AR: no customer_charge_minor — skipping invoice line"
            );
            mark_event_processed(pool, event_id, "shipping_receiving.shipping_cost.incurred", "ar-shipping-cost-consumer").await?;
            return Ok(());
        }
    };

    // Skip if there is no order reference.
    let order_ref = match payload.order_ref.as_deref() {
        Some(r) if !r.is_empty() => r,
        _ => {
            tracing::debug!(
                event_id = %event_id,
                tracking_number = %payload.tracking_number,
                "AR: no order_ref — skipping invoice line"
            );
            mark_event_processed(pool, event_id, "shipping_receiving.shipping_cost.incurred", "ar-shipping-cost-consumer").await?;
            return Ok(());
        }
    };

    // Find the invoice by order_ref (matches tilled_invoice_id or correlation_id).
    let invoice_row: Option<(i32, String, String)> = sqlx::query_as(
        r#"SELECT id, status, app_id
           FROM ar_invoices
           WHERE app_id = $1
             AND (tilled_invoice_id = $2 OR correlation_id = $2)
           LIMIT 1"#,
    )
    .bind(&payload.tenant_id)
    .bind(order_ref)
    .fetch_optional(pool)
    .await?;

    let (invoice_id, status, app_id) = match invoice_row {
        Some(r) => r,
        None => {
            tracing::warn!(
                event_id = %event_id,
                order_ref = %order_ref,
                tenant_id = %payload.tenant_id,
                "AR: invoice not found for order_ref — skipping shipping line"
            );
            mark_event_processed(pool, event_id, "shipping_receiving.shipping_cost.incurred", "ar-shipping-cost-consumer").await?;
            return Ok(());
        }
    };

    let line_description = format!(
        "Shipping — {} {}",
        payload.carrier_code, payload.tracking_number
    );
    let new_line = serde_json::json!({
        "description": line_description,
        "amount_cents": customer_charge,
        "quantity": 1
    });

    if is_mutable_status(&status) {
        // Append line and update total atomically.
        sqlx::query(
            r#"UPDATE ar_invoices
               SET line_item_details = COALESCE(line_item_details, '[]'::jsonb) || $1::jsonb,
                   amount_cents = amount_cents + $2,
                   updated_at = NOW()
               WHERE id = $3 AND app_id = $4"#,
        )
        .bind(serde_json::json!([new_line]))
        .bind(customer_charge)
        .bind(invoice_id)
        .bind(&app_id)
        .execute(pool)
        .await?;

        tracing::info!(
            event_id = %event_id,
            invoice_id = invoice_id,
            customer_charge_minor = customer_charge,
            "AR: shipping line added to invoice"
        );
    } else {
        // Invoice is posted/immutable — emit a warning event to the outbox.
        let warning = serde_json::json!({
            "invoice_id": invoice_id,
            "order_ref": order_ref,
            "tracking_number": payload.tracking_number,
            "carrier_code": payload.carrier_code,
            "customer_charge_minor": customer_charge,
            "currency": payload.currency,
            "reason": format!("Invoice is in status '{}' and cannot be modified; issue a supplementary invoice", status),
        });

        sqlx::query(
            r#"INSERT INTO events_outbox
               (event_id, event_type, aggregate_type, aggregate_id, payload)
               VALUES ($1, $2, 'invoice', $3, $4)"#,
        )
        .bind(Uuid::new_v4())
        .bind(EVENT_TYPE_CUSTOMER_CHARGE_AFTER_POST)
        .bind(invoice_id.to_string())
        .bind(&warning)
        .execute(pool)
        .await?;

        tracing::warn!(
            event_id = %event_id,
            invoice_id = invoice_id,
            status = %status,
            customer_charge_minor = customer_charge,
            "AR: invoice is immutable — customer_charge_after_post warning emitted"
        );
    }

    mark_event_processed(pool, event_id, "shipping_receiving.shipping_cost.incurred", "ar-shipping-cost-consumer").await?;

    Ok(())
}

// ============================================================================
// NATS consumer (production entry point)
// ============================================================================

/// Start the AR shipping cost consumer task.
///
/// Subscribes to `shipping_receiving.shipping_cost.incurred` and appends
/// shipping lines to open AR invoices when a customer charge is present.
pub fn start_shipping_cost_consumer(bus: Arc<dyn EventBus>, pool: PgPool) {
    tokio::spawn(async move {
        tracing::info!("AR: starting shipping_cost.incurred consumer");

        let subject = "shipping_receiving.shipping_cost.incurred";
        let mut stream = match bus.subscribe(subject).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(subject, error = %e, "AR: failed to subscribe to shipping cost events");
                return;
            }
        };

        tracing::info!(subject, "AR: subscribed to shipping cost events");

        while let Some(msg) = stream.next().await {
            let pool_ref = pool.clone();
            if let Err(e) = process_shipping_cost_message(&pool_ref, &msg).await {
                tracing::error!(error = %e, "AR: failed to process shipping_cost.incurred");
            }
        }

        tracing::warn!("AR: shipping_cost.incurred consumer stopped");
    });
}

// ============================================================================
// Helpers
// ============================================================================

fn is_mutable_status(status: &str) -> bool {
    matches!(status, "draft" | "open")
}

async fn process_shipping_cost_message(
    pool: &PgPool,
    msg: &BusMessage,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let envelope: EventEnvelope<ShippingCostIncurredPayload> =
        serde_json::from_slice(&msg.payload)
            .map_err(|e| format!("Failed to parse shipping_cost.incurred envelope: {}", e))?;

    tracing::info!(
        event_id = %envelope.event_id,
        tenant_id = %envelope.tenant_id,
        tracking_number = %envelope.payload.tracking_number,
        "AR: processing shipping_cost.incurred"
    );

    handle_shipping_cost_incurred(pool, envelope.event_id, &envelope.payload).await
}
