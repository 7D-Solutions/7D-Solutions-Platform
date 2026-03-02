//! AP consumer for inventory.item_received events.
//!
//! Ingests goods receipts into AP's po_receipt_links table for 3-way match.
//! This is a read-side linkage: no inventory DB writes.
//!
//! ## Idempotency
//! The service layer uses INSERT … ON CONFLICT (po_line_id, receipt_id) DO NOTHING.
//! Replaying the same event multiple times is safe.
//!
//! ## PO Line Inference
//! inventory.item_received carries purchase_order_id but not po_line_id.
//! For single-line POs, AP infers the link automatically.
//! For multi-line POs, the event is skipped (logged as warning); explicit receipt
//! link API (future bead) must be used instead.

use chrono::{DateTime, Utc};
use event_bus::{BusMessage, EventBus, EventEnvelope};
use futures::StreamExt;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::receipts_link::{
    service::ingest_receipt_link, IngestReceiptLinkRequest, ReceiptLinkError,
};

// ============================================================================
// Local payload mirror (anti-corruption layer)
// mirrors inventory::events::contracts::ItemReceivedPayload
// ============================================================================

#[derive(Debug, Clone, serde::Deserialize)]
pub struct InventoryItemReceivedPayload {
    /// Stable business key for this receipt line (idempotency anchor → receipt_id)
    pub receipt_line_id: Uuid,
    pub tenant_id: String,
    pub item_id: Uuid,
    pub sku: String,
    pub warehouse_id: Uuid,
    pub quantity: i64,
    pub unit_cost_minor: i64,
    pub currency: String,
    /// Source purchase order, if applicable
    pub purchase_order_id: Option<Uuid>,
    pub received_at: DateTime<Utc>,
}

// ============================================================================
// Minimal PO line view (AP-internal)
// ============================================================================

#[derive(Debug, sqlx::FromRow)]
struct PoLineView {
    pub line_id: Uuid,
    pub unit_of_measure: String,
    pub unit_price_minor: i64,
    pub gl_account_code: String,
}

// ============================================================================
// Public processing function (testable without NATS)
// ============================================================================

/// Process a single inventory.item_received event payload.
///
/// Looks up the PO in AP's own DB to resolve vendor_id and po_line_id.
/// For multi-line POs the link cannot be auto-inferred; logs a warning and returns Ok.
/// For missing POs, logs a warning and returns Ok (may not be AP-managed).
pub async fn handle_item_received(
    pool: &PgPool,
    _event_id: Uuid,
    payload: &InventoryItemReceivedPayload,
) -> Result<(), ReceiptLinkError> {
    let po_id = match payload.purchase_order_id {
        Some(id) => id,
        None => {
            tracing::debug!(
                receipt_line_id = %payload.receipt_line_id,
                "inventory.item_received has no purchase_order_id; skipping AP linkage"
            );
            return Ok(());
        }
    };

    // Look up vendor_id from AP's own purchase_orders table (no cross-module read)
    let vendor_id: Option<Uuid> =
        sqlx::query_scalar("SELECT vendor_id FROM purchase_orders WHERE po_id = $1")
            .bind(po_id)
            .fetch_optional(pool)
            .await?;

    let vendor_id = match vendor_id {
        Some(v) => v,
        None => {
            tracing::warn!(
                po_id = %po_id,
                receipt_line_id = %payload.receipt_line_id,
                "PO not found in AP DB; skipping receipt linkage"
            );
            return Ok(());
        }
    };

    // Look up PO lines from AP's own po_lines table
    let lines: Vec<PoLineView> = sqlx::query_as(
        r#"SELECT line_id, unit_of_measure, unit_price_minor, gl_account_code
           FROM po_lines WHERE po_id = $1 ORDER BY created_at ASC"#,
    )
    .bind(po_id)
    .fetch_all(pool)
    .await?;

    let line = match lines.as_slice() {
        [] => {
            tracing::warn!(po_id = %po_id, "PO has no lines; skipping receipt linkage");
            return Ok(());
        }
        [single] => single,
        _ => {
            tracing::warn!(
                po_id = %po_id,
                line_count = lines.len(),
                "Multi-line PO: cannot auto-infer receipt link from inventory.item_received; \
                 use the explicit receipt link API"
            );
            return Ok(());
        }
    };

    let req = IngestReceiptLinkRequest {
        po_id,
        po_line_id: line.line_id,
        vendor_id,
        receipt_id: payload.receipt_line_id,
        quantity_received: payload.quantity as f64,
        unit_of_measure: line.unit_of_measure.clone(),
        unit_price_minor: line.unit_price_minor,
        currency: payload.currency.clone(),
        gl_account_code: line.gl_account_code.clone(),
        received_at: payload.received_at,
        received_by: "system:inventory-consumer".to_string(),
    };

    ingest_receipt_link(pool, &req).await
}

// ============================================================================
// NATS consumer (production entry point)
// ============================================================================

/// Start the AP inventory receipt consumer task.
///
/// Subscribes to `inventory.item_received` and persists PO receipt links
/// via `handle_item_received`. Idempotent on redelivery.
pub async fn start_inventory_item_received_consumer(bus: Arc<dyn EventBus>, pool: PgPool) {
    tokio::spawn(async move {
        tracing::info!("AP: starting inventory.item_received consumer");

        let subject = "inventory.item_received";
        let mut stream = match bus.subscribe(subject).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(subject, error = %e, "AP: failed to subscribe");
                return;
            }
        };

        tracing::info!(subject, "AP: subscribed to inventory receipt events");

        while let Some(msg) = stream.next().await {
            let pool_ref = pool.clone();
            if let Err(e) = process_item_received_message(&pool_ref, &msg).await {
                tracing::error!(error = %e, "AP: failed to process inventory.item_received");
            }
        }

        tracing::warn!("AP: inventory.item_received consumer stopped");
    });
}

// ============================================================================
// Internal message processing
// ============================================================================

async fn process_item_received_message(
    pool: &PgPool,
    msg: &BusMessage,
) -> Result<(), Box<dyn std::error::Error>> {
    let envelope: EventEnvelope<InventoryItemReceivedPayload> =
        serde_json::from_slice(&msg.payload)
            .map_err(|e| format!("Failed to parse inventory.item_received envelope: {}", e))?;

    tracing::info!(
        event_id = %envelope.event_id,
        tenant_id = %envelope.tenant_id,
        receipt_line_id = %envelope.payload.receipt_line_id,
        sku = %envelope.payload.sku,
        "AP: processing inventory.item_received"
    );

    handle_item_received(pool, envelope.event_id, &envelope.payload)
        .await
        .map_err(|e| format!("handle_item_received failed: {}", e).into())
}

// ============================================================================
// Integrated Tests (real DB, no mocks)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serial_test::serial;

    const TEST_TENANT: &str = "test-tenant-inv-consumer";

    fn test_db_url() -> String {
        std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://ap_user:ap_pass@localhost:5443/ap_db".to_string())
    }

    async fn test_pool() -> PgPool {
        PgPool::connect(&test_db_url())
            .await
            .expect("Failed to connect to AP test DB")
    }

    async fn setup_fixtures(pool: &PgPool) -> (Uuid, Uuid, Uuid) {
        let vendor_id = Uuid::new_v4();
        sqlx::query(
            r#"INSERT INTO vendors (vendor_id, tenant_id, name, currency, payment_terms_days,
               is_active, created_at, updated_at)
               VALUES ($1, $2, $3, 'USD', 30, TRUE, NOW(), NOW())"#,
        )
        .bind(vendor_id)
        .bind(TEST_TENANT)
        .bind(format!("ConsumerVendor-{}", vendor_id))
        .execute(pool)
        .await
        .expect("insert vendor failed");

        let po_id = Uuid::new_v4();
        sqlx::query(
            r#"INSERT INTO purchase_orders
               (po_id, tenant_id, vendor_id, po_number, currency,
                total_minor, status, created_by, created_at)
               VALUES ($1, $2, $3, $4, 'USD', 5000, 'approved', 'system', NOW())"#,
        )
        .bind(po_id)
        .bind(TEST_TENANT)
        .bind(vendor_id)
        .bind(format!("PO-CON-{}", &po_id.to_string()[..8]))
        .execute(pool)
        .await
        .expect("insert PO failed");

        let line_id = Uuid::new_v4();
        sqlx::query(
            r#"INSERT INTO po_lines
               (line_id, po_id, description, quantity, unit_of_measure,
                unit_price_minor, line_total_minor, gl_account_code, created_at)
               VALUES ($1, $2, 'Widgets', 5.0, 'each', 1000, 5000, '6100', NOW())"#,
        )
        .bind(line_id)
        .bind(po_id)
        .execute(pool)
        .await
        .expect("insert PO line failed");

        (vendor_id, po_id, line_id)
    }

    async fn cleanup(pool: &PgPool) {
        sqlx::query(
            "DELETE FROM po_receipt_links WHERE po_id IN \
             (SELECT po_id FROM purchase_orders WHERE tenant_id = $1)",
        )
        .bind(TEST_TENANT)
        .execute(pool)
        .await
        .ok();

        sqlx::query(
            "DELETE FROM po_lines WHERE po_id IN \
             (SELECT po_id FROM purchase_orders WHERE tenant_id = $1)",
        )
        .bind(TEST_TENANT)
        .execute(pool)
        .await
        .ok();

        sqlx::query(
            "DELETE FROM po_status WHERE po_id IN \
             (SELECT po_id FROM purchase_orders WHERE tenant_id = $1)",
        )
        .bind(TEST_TENANT)
        .execute(pool)
        .await
        .ok();

        sqlx::query(
            "DELETE FROM events_outbox WHERE aggregate_type = 'po' \
             AND aggregate_id IN \
             (SELECT po_id::TEXT FROM purchase_orders WHERE tenant_id = $1)",
        )
        .bind(TEST_TENANT)
        .execute(pool)
        .await
        .ok();

        sqlx::query("DELETE FROM purchase_orders WHERE tenant_id = $1")
            .bind(TEST_TENANT)
            .execute(pool)
            .await
            .ok();

        sqlx::query(
            "DELETE FROM events_outbox WHERE aggregate_type = 'vendor' \
             AND aggregate_id IN \
             (SELECT vendor_id::TEXT FROM vendors WHERE tenant_id = $1)",
        )
        .bind(TEST_TENANT)
        .execute(pool)
        .await
        .ok();

        sqlx::query("DELETE FROM vendors WHERE tenant_id = $1")
            .bind(TEST_TENANT)
            .execute(pool)
            .await
            .ok();
    }

    fn sample_payload(po_id: Option<Uuid>) -> InventoryItemReceivedPayload {
        InventoryItemReceivedPayload {
            receipt_line_id: Uuid::new_v4(),
            tenant_id: TEST_TENANT.to_string(),
            item_id: Uuid::new_v4(),
            sku: "WIDGET-001".to_string(),
            warehouse_id: Uuid::new_v4(),
            quantity: 5,
            unit_cost_minor: 1000,
            currency: "USD".to_string(),
            purchase_order_id: po_id,
            received_at: Utc::now(),
        }
    }

    #[tokio::test]
    #[serial]
    async fn test_handle_single_line_po_creates_link() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let (_vendor_id, po_id, _line_id) = setup_fixtures(&pool).await;
        let payload = sample_payload(Some(po_id));
        let receipt_id = payload.receipt_line_id;

        handle_item_received(&pool, Uuid::new_v4(), &payload)
            .await
            .expect("handle failed");

        let (count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM po_receipt_links WHERE po_id = $1 AND receipt_id = $2",
        )
        .bind(po_id)
        .bind(receipt_id)
        .fetch_one(&pool)
        .await
        .expect("count query failed");

        assert_eq!(count, 1);
        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_handle_idempotent_on_redelivery() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let (_vendor_id, po_id, _line_id) = setup_fixtures(&pool).await;
        let payload = sample_payload(Some(po_id));
        let receipt_id = payload.receipt_line_id;

        handle_item_received(&pool, Uuid::new_v4(), &payload)
            .await
            .expect("first handle failed");
        handle_item_received(&pool, Uuid::new_v4(), &payload)
            .await
            .expect("second handle must not error");

        let (count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM po_receipt_links WHERE po_id = $1 AND receipt_id = $2",
        )
        .bind(po_id)
        .bind(receipt_id)
        .fetch_one(&pool)
        .await
        .expect("count query failed");

        assert_eq!(count, 1, "idempotent: redelivery must not duplicate rows");
        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_handle_no_po_id_skips_gracefully() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let payload = sample_payload(None); // no purchase_order_id

        handle_item_received(&pool, Uuid::new_v4(), &payload)
            .await
            .expect("handle with no PO must return Ok");

        let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM po_receipt_links")
            .fetch_one(&pool)
            .await
            .expect("count query failed");
        assert_eq!(count, 0);
        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_handle_unknown_po_id_skips_gracefully() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let payload = sample_payload(Some(Uuid::new_v4())); // unknown PO

        handle_item_received(&pool, Uuid::new_v4(), &payload)
            .await
            .expect("handle with unknown PO must return Ok");
        cleanup(&pool).await;
    }
}
