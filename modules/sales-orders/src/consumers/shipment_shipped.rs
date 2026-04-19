//! Consumer for shipping_receiving.outbound_shipped events.
//! Updates shipped_qty on matching SO lines, emits invoice_requested per line,
//! and closes the SO when all lines are invoiced.

use chrono::Utc;
use event_bus::EventBus;
use futures::StreamExt;
use serde::Deserialize;
use sqlx::PgPool;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::orders::{repo, SoStatus};
use crate::events::{
    build_invoice_requested_envelope, InvoiceRequestedPayload, EVENT_TYPE_INVOICE_REQUESTED,
};
use crate::outbox::enqueue_event_tx;

const SUBJECT: &str = "shipping_receiving.outbound_shipped";
#[allow(dead_code)]
const QUEUE_GROUP: &str = "sales-orders-outbound-shipped";

// ── Incoming payload structs (mirror shipping_receiving.outbound_shipped) ─────

#[derive(Debug, Deserialize)]
struct OutboundShippedLine {
    #[allow(dead_code)]
    pub line_id: Uuid,
    pub sku: String,
    pub qty_shipped: i64,
    #[serde(default)]
    pub source_ref_type: Option<String>,
    #[serde(default)]
    pub source_ref_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
struct OutboundShippedPayload {
    pub tenant_id: String,
    #[allow(dead_code)]
    pub shipment_id: Uuid,
    pub lines: Vec<OutboundShippedLine>,
}

#[derive(Deserialize)]
struct EventEnvelopeShim<T> {
    pub payload: T,
}

// ── Consumer ──────────────────────────────────────────────────────────────────

pub fn start_shipment_shipped_consumer(bus: Arc<dyn EventBus>, pool: PgPool) {
    tokio::spawn(async move {
        consume(bus, pool).await;
    });
}

async fn consume(bus: Arc<dyn EventBus>, pool: PgPool) {
    let mut stream = match bus.subscribe(SUBJECT).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("SO: failed to subscribe to {}: {}", SUBJECT, e);
            return;
        }
    };

    while let Some(msg) = stream.next().await {
        if let Err(e) = process_message(&msg, &pool).await {
            tracing::error!("SO: outbound_shipped processing error: {}", e);
        }
    }
}

async fn process_message(
    msg: &event_bus::BusMessage,
    pool: &PgPool,
) -> Result<(), Box<dyn std::error::Error>> {
    let shim: EventEnvelopeShim<OutboundShippedPayload> = serde_json::from_slice(&msg.payload)?;
    let payload = shim.payload;

    // Group shipment lines by sales order UUID.
    let mut so_groups: HashMap<Uuid, Vec<&OutboundShippedLine>> = HashMap::new();
    for line in &payload.lines {
        if line.source_ref_type.as_deref() == Some("sales_order") {
            if let Some(so_id) = line.source_ref_id {
                so_groups.entry(so_id).or_default().push(line);
            }
        }
    }

    for (so_id, shipped_lines) in so_groups {
        if let Err(e) = process_so_lines(pool, &payload.tenant_id, so_id, &shipped_lines).await {
            tracing::error!("SO: failed processing outbound_shipped for SO {}: {}", so_id, e);
        }
    }

    Ok(())
}

async fn process_so_lines(
    pool: &PgPool,
    tenant_id: &str,
    so_id: Uuid,
    shipped_lines: &[&OutboundShippedLine],
) -> Result<(), Box<dyn std::error::Error>> {
    let order = match repo::fetch_order_for_mutation(pool, so_id, tenant_id).await? {
        Some(o) => o,
        None => {
            tracing::warn!("SO: outbound_shipped references unknown SO {}", so_id);
            return Ok(());
        }
    };
    let so_lines = repo::fetch_lines_for_order(pool, so_id, tenant_id).await?;

    let mut tx = pool.begin().await?;
    let mut just_invoiced: HashSet<Uuid> = HashSet::new();

    for shipped in shipped_lines {
        let Some(so_line) = so_lines
            .iter()
            .find(|l| l.part_number.as_deref() == Some(shipped.sku.as_str()))
        else {
            tracing::warn!(
                "SO: no line with part_number={} in SO {}",
                shipped.sku,
                so_id
            );
            continue;
        };

        repo::update_line_shipped_qty(&mut *tx, so_line.id, tenant_id, shipped.qty_shipped as f64)
            .await?;
        repo::mark_line_invoiced(&mut *tx, so_line.id, tenant_id).await?;
        just_invoiced.insert(so_line.id);

        let event_id = Uuid::new_v4();
        let envelope = build_invoice_requested_envelope(
            event_id,
            tenant_id.to_string(),
            Uuid::new_v4().to_string(),
            None,
            InvoiceRequestedPayload {
                sales_order_id: so_id,
                line_id: so_line.id,
                customer_id: order.customer_id,
                amount_cents: so_line.unit_price_cents * shipped.qty_shipped,
                currency: order.currency.clone(),
                tenant_id: tenant_id.to_string(),
                requested_at: Utc::now(),
            },
        );
        enqueue_event_tx(
            &mut tx,
            event_id,
            EVENT_TYPE_INVOICE_REQUESTED,
            "sales_order",
            &so_id.to_string(),
            &envelope,
        )
        .await?;
    }

    // Close the order when every line is now invoiced.
    let all_invoiced = !so_lines.is_empty()
        && so_lines
            .iter()
            .all(|l| l.invoiced_at.is_some() || just_invoiced.contains(&l.id));

    if all_invoiced {
        let current = SoStatus::from_str(&order.status).unwrap_or(SoStatus::Shipped);
        if current.can_transition_to(SoStatus::Closed) {
            repo::update_order_status(&mut *tx, so_id, tenant_id, SoStatus::Closed.as_str())
                .await?;
            tracing::info!("SO: order {} closed — all lines invoiced", so_id);
        }
    }

    tx.commit().await?;
    Ok(())
}
