//! Ingestion handler for `ar.events.ar.invoice_opened`.
//!
//! Upserts into `rpt_open_invoices_cache` with status='open'.
//! Replay-safe via ON CONFLICT DO UPDATE on (tenant_id, invoice_id).

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use sqlx::PgPool;

use crate::ingest::StreamHandler;

/// NATS subject emitted by the AR outbox publisher.
pub const SUBJECT_INVOICE_OPENED: &str = "ar.events.ar.invoice_opened";

/// Consumer name for checkpoint tracking.
pub const CONSUMER_INVOICE_OPENED: &str = "reporting.invoice_opened";

// ── Local payload mirror (no cross-module import) ───────────────────────────

#[derive(Debug, Deserialize)]
struct InvoiceOpenedPayload {
    invoice_id: String,
    customer_id: String,
    amount_cents: i64,
    currency: String,
    created_at: DateTime<Utc>,
    due_at: Option<DateTime<Utc>>,
}

// ── Handler ─────────────────────────────────────────────────────────────────

pub struct InvoiceOpenedHandler;

#[async_trait]
impl StreamHandler for InvoiceOpenedHandler {
    async fn handle(
        &self,
        pool: &PgPool,
        tenant_id: &str,
        _event_id: &str,
        payload: &serde_json::Value,
    ) -> Result<(), anyhow::Error> {
        let p = InvoiceOpenedPayload::deserialize(payload)
            .map_err(|e| anyhow::anyhow!("Failed to parse invoice_opened payload: {}", e))?;

        sqlx::query(
            r#"
            INSERT INTO rpt_open_invoices_cache
                (tenant_id, invoice_id, customer_id, currency,
                 amount_cents, issued_at, due_at, status, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, 'open', NOW(), NOW())
            ON CONFLICT (tenant_id, invoice_id) DO UPDATE SET
                customer_id  = EXCLUDED.customer_id,
                currency     = EXCLUDED.currency,
                amount_cents = EXCLUDED.amount_cents,
                issued_at    = EXCLUDED.issued_at,
                due_at       = EXCLUDED.due_at,
                updated_at   = NOW()
            "#,
        )
        .bind(tenant_id)
        .bind(&p.invoice_id)
        .bind(&p.customer_id)
        .bind(&p.currency)
        .bind(p.amount_cents)
        .bind(p.created_at)
        .bind(p.due_at)
        .execute(pool)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to upsert open invoice cache: {}", e))?;

        Ok(())
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::IngestConsumer;
    use event_bus::BusMessage;
    use serial_test::serial;
    use std::sync::Arc;

    const TENANT: &str = "test-inv-opened-tenant";

    fn test_db_url() -> String {
        std::env::var("REPORTING_DATABASE_URL")
            .unwrap_or_else(|_| "postgres://ap_user:ap_pass@localhost:5443/reporting_test".into())
    }

    async fn test_pool() -> PgPool {
        let pool = PgPool::connect(&test_db_url()).await.expect("connect");
        sqlx::migrate!("./db/migrations").run(&pool).await.expect("migrate");
        pool
    }

    async fn cleanup(pool: &PgPool) {
        sqlx::query("DELETE FROM rpt_open_invoices_cache WHERE tenant_id = $1")
            .bind(TENANT)
            .execute(pool)
            .await
            .ok();
        sqlx::query("DELETE FROM rpt_ingestion_checkpoints WHERE tenant_id = $1")
            .bind(TENANT)
            .execute(pool)
            .await
            .ok();
    }

    fn make_opened_envelope(event_id: &str, invoice_id: &str, customer_id: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "event_id": event_id,
            "tenant_id": TENANT,
            "event_type": "ar.invoice_opened",
            "source_module": "ar",
            "data": {
                "invoice_id": invoice_id,
                "customer_id": customer_id,
                "app_id": "test-app",
                "amount_cents": 50000,
                "currency": "USD",
                "created_at": "2026-02-15T12:00:00Z",
                "due_at": "2026-03-15T12:00:00Z",
                "paid_at": null
            }
        }))
        .unwrap()
    }

    #[tokio::test]
    #[serial]
    async fn test_invoice_opened_creates_cache_entry() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let handler = Arc::new(InvoiceOpenedHandler);
        let consumer = IngestConsumer::new("test-inv-opened-basic", pool.clone(), handler);

        let msg = BusMessage::new(
            SUBJECT_INVOICE_OPENED.to_string(),
            make_opened_envelope("evt-open-001", "inv-001", "cust-001"),
        );

        let processed = consumer.process_message(&msg).await.expect("process");
        assert!(processed);

        let (status, amount): (String, i64) = sqlx::query_as(
            "SELECT status, amount_cents FROM rpt_open_invoices_cache \
             WHERE tenant_id = $1 AND invoice_id = $2",
        )
        .bind(TENANT)
        .bind("inv-001")
        .fetch_one(&pool)
        .await
        .expect("fetch");

        assert_eq!(status, "open");
        assert_eq!(amount, 50000);

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_invoice_opened_replay_safe() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let handler = Arc::new(InvoiceOpenedHandler);

        // First delivery
        let c1 = IngestConsumer::new("test-inv-opened-replay-1", pool.clone(), handler.clone());
        let msg = BusMessage::new(
            SUBJECT_INVOICE_OPENED.to_string(),
            make_opened_envelope("evt-open-r1", "inv-replay", "cust-001"),
        );
        c1.process_message(&msg).await.expect("first");

        // Second delivery with different consumer (simulates checkpoint reset)
        let c2 = IngestConsumer::new("test-inv-opened-replay-2", pool.clone(), handler);
        let msg2 = BusMessage::new(
            SUBJECT_INVOICE_OPENED.to_string(),
            make_opened_envelope("evt-open-r2", "inv-replay", "cust-001"),
        );
        c2.process_message(&msg2).await.expect("second");

        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM rpt_open_invoices_cache \
             WHERE tenant_id = $1 AND invoice_id = $2",
        )
        .bind(TENANT)
        .bind("inv-replay")
        .fetch_one(&pool)
        .await
        .expect("count");

        assert_eq!(count, 1, "must have exactly one row despite two deliveries");

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_invoice_opened_checkpoint_dedup() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let handler = Arc::new(InvoiceOpenedHandler);
        let consumer = IngestConsumer::new("test-inv-opened-dedup", pool.clone(), handler);

        let msg = BusMessage::new(
            SUBJECT_INVOICE_OPENED.to_string(),
            make_opened_envelope("evt-open-dup", "inv-dup", "cust-001"),
        );

        let first = consumer.process_message(&msg).await.expect("first");
        assert!(first, "first delivery processed");

        let second = consumer.process_message(&msg).await.expect("second");
        assert!(!second, "duplicate must be skipped by checkpoint");

        cleanup(&pool).await;
    }
}
