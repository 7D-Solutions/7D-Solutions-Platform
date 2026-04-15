//! AP consumer for docmgmt.attachment.created events.
//!
//! Filters for AP-relevant entity types (ap_bill, ap_invoice), validates the
//! referenced bill exists, records the link in bill_attachments, and emits an
//! ap.bill.attachment_linked outbox event.
//!
//! ## Idempotency
//! UNIQUE constraint on (bill_id, attachment_id) in bill_attachments ensures
//! redelivered events are safe to replay.
//!
//! ## Filtering
//! entity_type values not in ("ap_bill", "ap_invoice") are silently ignored.
//! This consumer does not care about attachments on other entity types.

use event_bus::{BusMessage, EventBus};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

use crate::outbox::enqueue_event_tx;

// ============================================================================
// Inbound event shape (mirrors doc-mgmt AttachmentCreatedPayload)
// ============================================================================

#[derive(Debug, Deserialize)]
struct AttachmentCreatedPayload {
    pub tenant_id: String,
    pub attachment_id: Uuid,
    pub entity_type: String,
    pub entity_id: String,
    pub filename: String,
    pub mime_type: String,
    pub size_bytes: i64,
    pub uploaded_by: Uuid,
}

// ============================================================================
// Outbound event payload
// ============================================================================

#[derive(Debug, Serialize)]
struct BillAttachmentLinkedPayload {
    pub bill_id: Uuid,
    pub attachment_id: Uuid,
    pub tenant_id: String,
}

// ============================================================================
// AP-relevant entity types
// ============================================================================

const AP_ENTITY_TYPES: &[&str] = &["ap_bill", "ap_invoice"];

// ============================================================================
// Core processing (testable without NATS)
// ============================================================================

/// Process a single `docmgmt.attachment.created` NATS message.
///
/// Returns `Ok(())` for messages that are safely ignored (wrong entity type,
/// unknown bill). Returns `Err` only for transient failures that should cause
/// the consumer to log an error (DB errors, serialization failures).
pub async fn handle_attachment_created(
    pool: &PgPool,
    msg: &BusMessage,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let payload: AttachmentCreatedPayload = serde_json::from_slice(&msg.payload)
        .map_err(|e| format!("ap_attachment_consumer: failed to deserialize event: {e}"))?;

    // ── Filter: only AP-relevant entity types ────────────────────────────────
    if !AP_ENTITY_TYPES.contains(&payload.entity_type.as_str()) {
        tracing::debug!(
            entity_type = %payload.entity_type,
            attachment_id = %payload.attachment_id,
            "ap_attachment_consumer: ignoring non-AP entity type"
        );
        return Ok(());
    }

    // ── Resolve entity_id as bill_id ─────────────────────────────────────────
    let bill_id: Uuid = match payload.entity_id.parse() {
        Ok(id) => id,
        Err(_) => {
            tracing::warn!(
                entity_id = %payload.entity_id,
                attachment_id = %payload.attachment_id,
                "ap_attachment_consumer: entity_id is not a valid UUID — skipping"
            );
            return Ok(());
        }
    };

    // ── Guard: bill must exist in AP ──────────────────────────────────────────
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM vendor_bills WHERE bill_id = $1 AND tenant_id = $2)",
    )
    .bind(bill_id)
    .bind(&payload.tenant_id)
    .fetch_one(pool)
    .await?;

    if !exists {
        tracing::warn!(
            bill_id = %bill_id,
            attachment_id = %payload.attachment_id,
            tenant_id = %payload.tenant_id,
            "ap_attachment_consumer: bill not found — skipping attachment link"
        );
        return Ok(());
    }

    // ── Mutation + Outbox (atomic) ────────────────────────────────────────────
    let mut tx = pool.begin().await?;

    // INSERT OR IGNORE duplicate (idempotency via ON CONFLICT DO NOTHING)
    sqlx::query(
        "INSERT INTO bill_attachments (bill_id, attachment_id, tenant_id)
         VALUES ($1, $2, $3)
         ON CONFLICT (bill_id, attachment_id) DO NOTHING",
    )
    .bind(bill_id)
    .bind(payload.attachment_id)
    .bind(&payload.tenant_id)
    .execute(&mut *tx)
    .await?;

    let event_id = Uuid::new_v4();
    let linked_payload = BillAttachmentLinkedPayload {
        bill_id,
        attachment_id: payload.attachment_id,
        tenant_id: payload.tenant_id.clone(),
    };

    enqueue_event_tx(
        &mut tx,
        event_id,
        "ap.bill.attachment_linked",
        "bill",
        &bill_id.to_string(),
        &linked_payload,
    )
    .await?;

    tx.commit().await?;

    tracing::info!(
        bill_id = %bill_id,
        attachment_id = %payload.attachment_id,
        filename = %payload.filename,
        mime_type = %payload.mime_type,
        size_bytes = payload.size_bytes,
        uploaded_by = %payload.uploaded_by,
        "ap_attachment_consumer: bill attachment linked"
    );

    Ok(())
}

// ============================================================================
// NATS consumer worker
// ============================================================================

/// Start the AP docmgmt attachment consumer task.
///
/// Spawns a background tokio task that subscribes to `docmgmt.attachment.created`
/// and processes each event via `handle_attachment_created`. Errors are logged
/// but do not stop the worker. Returns the JoinHandle of the spawned task.
pub fn start_attachment_linked_consumer(
    bus: Arc<dyn EventBus>,
    pool: PgPool,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let subject = "docmgmt.attachment.created";
        let mut stream = match bus.subscribe(subject).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(subject, error = %e, "AP: failed to subscribe to attachment events");
                return;
            }
        };

        tracing::info!(subject, "AP: attachment_linked consumer started");

        while let Some(msg) = stream.next().await {
            let pool_ref = pool.clone();
            if let Err(e) = handle_attachment_created(&pool_ref, &msg).await {
                tracing::error!(error = %e, "AP: failed to process docmgmt.attachment.created");
            }
        }

        tracing::warn!("AP: attachment_linked consumer stopped");
    })
}

// ============================================================================
// Integrated Tests (real DB, no mocks)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use event_bus::BusMessage;
    use serial_test::serial;

    const TEST_TENANT: &str = "test-tenant-attachment-consumer";

    fn db_url() -> String {
        std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://ap_user:ap_pass@localhost:5443/ap_db".to_string())
    }

    async fn test_pool() -> PgPool {
        let pool = PgPool::connect(&db_url())
            .await
            .expect("connect to AP test DB");
        sqlx::migrate!("db/migrations")
            .run(&pool)
            .await
            .expect("run AP migrations");
        pool
    }

    async fn seed_bill(pool: &PgPool) -> (Uuid, Uuid) {
        let vendor_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO vendors (vendor_id, tenant_id, name, currency, payment_terms_days, is_active, created_at, updated_at)
             VALUES ($1, $2, $3, 'USD', 30, TRUE, NOW(), NOW())",
        )
        .bind(vendor_id)
        .bind(TEST_TENANT)
        .bind(format!("AttachVendor-{}", vendor_id))
        .execute(pool)
        .await
        .expect("insert vendor");

        let bill_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO vendor_bills (bill_id, tenant_id, vendor_id, vendor_invoice_ref, currency, total_minor, tax_minor, invoice_date, due_date, status, entered_by)
             VALUES ($1, $2, $3, $4, 'USD', 10000, 0, NOW(), NOW() + INTERVAL '30 days', 'open', 'system')",
        )
        .bind(bill_id)
        .bind(TEST_TENANT)
        .bind(vendor_id)
        .bind(format!("INV-{}", &bill_id.to_string()[..8]))
        .execute(pool)
        .await
        .expect("insert bill");

        (vendor_id, bill_id)
    }

    async fn cleanup(pool: &PgPool) {
        sqlx::query("DELETE FROM bill_attachments WHERE tenant_id = $1")
            .bind(TEST_TENANT)
            .execute(pool)
            .await
            .ok();
        sqlx::query(
            "DELETE FROM events_outbox WHERE aggregate_type = 'bill' \
             AND aggregate_id IN (SELECT bill_id::TEXT FROM vendor_bills WHERE tenant_id = $1)",
        )
        .bind(TEST_TENANT)
        .execute(pool)
        .await
        .ok();
        sqlx::query(
            "DELETE FROM events_outbox WHERE aggregate_type = 'vendor' \
             AND aggregate_id IN (SELECT vendor_id::TEXT FROM vendors WHERE tenant_id = $1)",
        )
        .bind(TEST_TENANT)
        .execute(pool)
        .await
        .ok();
        sqlx::query("DELETE FROM vendor_bills WHERE tenant_id = $1")
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

    fn make_msg(entity_type: &str, entity_id: &str, attachment_id: Uuid) -> BusMessage {
        let payload = serde_json::to_vec(&serde_json::json!({
            "tenant_id": TEST_TENANT,
            "attachment_id": attachment_id,
            "entity_type": entity_type,
            "entity_id": entity_id,
            "filename": "receipt.pdf",
            "mime_type": "application/pdf",
            "size_bytes": 4096_i64,
            "uploaded_by": Uuid::new_v4(),
        }))
        .expect("query failed");
        BusMessage::new("docmgmt.attachment.created".to_string(), payload)
    }

    #[tokio::test]
    #[serial]
    async fn test_ap_bill_creates_link_and_outbox_event() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let (_vendor_id, bill_id) = seed_bill(&pool).await;
        let attachment_id = Uuid::new_v4();

        let msg = make_msg("ap_bill", &bill_id.to_string(), attachment_id);
        handle_attachment_created(&pool, &msg)
            .await
            .expect("handle failed");

        // bill_attachments row created
        let (count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM bill_attachments WHERE bill_id = $1 AND attachment_id = $2",
        )
        .bind(bill_id)
        .bind(attachment_id)
        .fetch_one(&pool)
        .await
        .expect("query failed");
        assert_eq!(count, 1, "bill_attachments row must be created");

        // outbox event emitted
        let (evt_count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM events_outbox \
             WHERE event_type = 'ap.bill.attachment_linked' AND aggregate_id = $1",
        )
        .bind(bill_id.to_string())
        .fetch_one(&pool)
        .await
        .expect("query failed");
        assert_eq!(evt_count, 1, "outbox event must be emitted");

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_non_ap_entity_type_is_ignored() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let (_vendor_id, bill_id) = seed_bill(&pool).await;
        let attachment_id = Uuid::new_v4();

        let msg = make_msg("sales_order", &bill_id.to_string(), attachment_id);
        handle_attachment_created(&pool, &msg)
            .await
            .expect("handle must not error");

        let (count,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM bill_attachments WHERE attachment_id = $1")
                .bind(attachment_id)
                .fetch_one(&pool)
                .await
                .expect("query failed");
        assert_eq!(count, 0, "non-AP entity types must be ignored");

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_unknown_bill_id_skips_gracefully() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let attachment_id = Uuid::new_v4();
        let unknown_bill_id = Uuid::new_v4();

        let msg = make_msg("ap_bill", &unknown_bill_id.to_string(), attachment_id);
        handle_attachment_created(&pool, &msg)
            .await
            .expect("must not error for unknown bill");

        let (count,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM bill_attachments WHERE attachment_id = $1")
                .bind(attachment_id)
                .fetch_one(&pool)
                .await
                .expect("query failed");
        assert_eq!(count, 0, "unknown bill must not create a link");

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_idempotent_on_duplicate_event() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let (_vendor_id, bill_id) = seed_bill(&pool).await;
        let attachment_id = Uuid::new_v4();

        let msg = make_msg("ap_bill", &bill_id.to_string(), attachment_id);
        handle_attachment_created(&pool, &msg)
            .await
            .expect("first handle failed");
        handle_attachment_created(&pool, &msg)
            .await
            .expect("second handle must not error");

        let (count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM bill_attachments WHERE bill_id = $1 AND attachment_id = $2",
        )
        .bind(bill_id)
        .bind(attachment_id)
        .fetch_one(&pool)
        .await
        .expect("query failed");
        assert_eq!(count, 1, "duplicate event must not create duplicate row");

        cleanup(&pool).await;
    }
}
