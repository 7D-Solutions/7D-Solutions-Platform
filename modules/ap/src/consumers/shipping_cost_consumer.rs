//! AP consumer for shipping_receiving.shipping_cost.incurred events.
//!
//! Creates an open (pending) vendor bill for each carrier shipping cost.
//! The bill serves as the AP obligation that will be matched when the
//! carrier's weekly/monthly invoice arrives.
//!
//! ## Idempotency
//! Uses the UNIQUE constraint on (tenant_id, vendor_id, vendor_invoice_ref) in
//! vendor_bills. tracking_number is used as vendor_invoice_ref. A duplicate
//! event delivery reuses ON CONFLICT DO NOTHING — safe to replay.
//!
//! ## Carrier mapping
//! carrier_code → vendor_id resolved from ap_carrier_vendor_mapping per tenant.
//! If no mapping exists the event is logged and skipped (carrier not configured in AP).

use chrono::Utc;
use event_bus::{BusMessage, EventBus, EventEnvelope};
use futures::StreamExt;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

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
    pub charge_minor: i64,
    pub currency: String,
    pub incurred_at: chrono::DateTime<Utc>,
}

// ============================================================================
// Public processing function (testable without NATS)
// ============================================================================

/// Process a single shipping_cost.incurred event payload.
///
/// Looks up the carrier → vendor mapping. If not configured for this tenant,
/// logs a warning and returns Ok (the platform doesn't know about this carrier).
/// On success, inserts an open vendor_bill + bill_line. Idempotent on redelivery.
pub async fn handle_shipping_cost_incurred(
    pool: &PgPool,
    _event_id: Uuid,
    payload: &ShippingCostIncurredPayload,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Look up carrier → vendor mapping
    let mapping: Option<(Uuid, String)> = sqlx::query_as(
        r#"SELECT vendor_id, default_gl_account_code
           FROM ap_carrier_vendor_mapping
           WHERE tenant_id = $1 AND carrier_code = $2"#,
    )
    .bind(&payload.tenant_id)
    .bind(&payload.carrier_code)
    .fetch_optional(pool)
    .await?;

    let (vendor_id, gl_account_code) = match mapping {
        Some(m) => m,
        None => {
            tracing::warn!(
                tenant_id = %payload.tenant_id,
                carrier_code = %payload.carrier_code,
                tracking_number = %payload.tracking_number,
                "No AP carrier-vendor mapping found; skipping shipping cost obligation"
            );
            return Ok(());
        }
    };

    // Insert vendor_bill with tracking_number as vendor_invoice_ref.
    // ON CONFLICT DO NOTHING handles redelivery idempotency.
    let bill_id = Uuid::new_v4();
    let inserted: Option<(Uuid,)> = sqlx::query_as(
        r#"INSERT INTO vendor_bills
           (bill_id, tenant_id, vendor_id, vendor_invoice_ref, currency,
            total_minor, invoice_date, due_date, status, entered_by, entered_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $7 + INTERVAL '30 days', 'open',
                   'system:shipping-cost-consumer', NOW())
           ON CONFLICT (tenant_id, vendor_id, vendor_invoice_ref) DO NOTHING
           RETURNING bill_id"#,
    )
    .bind(bill_id)
    .bind(&payload.tenant_id)
    .bind(vendor_id)
    .bind(&payload.tracking_number)
    .bind(&payload.currency)
    .bind(payload.charge_minor)
    .bind(payload.incurred_at)
    .fetch_optional(pool)
    .await?;

    let actual_bill_id = match inserted {
        Some((id,)) => id,
        None => {
            tracing::debug!(
                tenant_id = %payload.tenant_id,
                tracking_number = %payload.tracking_number,
                "Shipping cost bill already exists — duplicate event ignored"
            );
            return Ok(());
        }
    };

    // Insert the single bill line for this shipping cost.
    let line_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO bill_lines
           (line_id, bill_id, description, quantity, unit_price_minor,
            line_total_minor, gl_account_code, created_at)
           VALUES ($1, $2, $3, 1.0, $4, $4, $5, NOW())"#,
    )
    .bind(line_id)
    .bind(actual_bill_id)
    .bind(format!(
        "Carrier shipping — {} {}",
        payload.carrier_code, payload.tracking_number
    ))
    .bind(payload.charge_minor)
    .bind(&gl_account_code)
    .execute(pool)
    .await?;

    tracing::info!(
        tenant_id = %payload.tenant_id,
        bill_id = %actual_bill_id,
        vendor_id = %vendor_id,
        carrier_code = %payload.carrier_code,
        tracking_number = %payload.tracking_number,
        charge_minor = payload.charge_minor,
        "AP: shipping cost obligation created"
    );

    Ok(())
}

// ============================================================================
// NATS consumer (production entry point)
// ============================================================================

/// Start the AP shipping cost consumer task.
///
/// Subscribes to `shipping_receiving.shipping_cost.incurred` and creates
/// carrier AP obligations via `handle_shipping_cost_incurred`.
pub fn start_shipping_cost_consumer(bus: Arc<dyn EventBus>, pool: PgPool) {
    tokio::spawn(async move {
        tracing::info!("AP: starting shipping_cost.incurred consumer");

        let subject = "shipping_receiving.shipping_cost.incurred";
        let mut stream = match bus.subscribe(subject).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(subject, error = %e, "AP: failed to subscribe to shipping cost events");
                return;
            }
        };

        tracing::info!(subject, "AP: subscribed to shipping cost events");

        while let Some(msg) = stream.next().await {
            let pool_ref = pool.clone();
            if let Err(e) = process_shipping_cost_message(&pool_ref, &msg).await {
                tracing::error!(error = %e, "AP: failed to process shipping_cost.incurred");
            }
        }

        tracing::warn!("AP: shipping_cost.incurred consumer stopped");
    });
}

// ============================================================================
// Internal message processing
// ============================================================================

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
        carrier_code = %envelope.payload.carrier_code,
        "AP: processing shipping_cost.incurred"
    );

    handle_shipping_cost_incurred(pool, envelope.event_id, &envelope.payload).await
}
