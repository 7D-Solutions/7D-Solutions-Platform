//! Ingestion handler for `ar.events.ar.invoice_paid`.
//!
//! Two writes per event:
//! (a) Upsert into rpt_payment_history (historical CDF data)
//! (b) Update rpt_open_invoices_cache status → 'paid'
//!
//! Replay-safe: both use ON CONFLICT / WHERE clauses that are idempotent.
//! Handles out-of-order: if paid arrives before opened, (b) is a no-op
//! (no open row exists yet). When the opened event arrives later it will
//! create the row with status='open', and the next paid replay will
//! transition it. The payment_history row is always written regardless.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use sqlx::PgPool;

use crate::ingest::StreamHandler;

/// NATS subject emitted by the AR outbox publisher.
pub const SUBJECT_INVOICE_PAID: &str = "ar.events.ar.invoice_paid";

/// Consumer name for checkpoint tracking.
pub const CONSUMER_INVOICE_PAID: &str = "reporting.invoice_paid";

// ── Local payload mirror (no cross-module import) ───────────────────────────

#[derive(Debug, Deserialize)]
struct InvoicePaidPayload {
    invoice_id: String,
    customer_id: String,
    amount_cents: i64,
    currency: String,
    created_at: DateTime<Utc>,
    paid_at: Option<DateTime<Utc>>,
}

// ── Handler ─────────────────────────────────────────────────────────────────

pub struct InvoicePaidHandler;

#[async_trait]
impl StreamHandler for InvoicePaidHandler {
    async fn handle(
        &self,
        pool: &PgPool,
        tenant_id: &str,
        _event_id: &str,
        payload: &serde_json::Value,
    ) -> Result<(), anyhow::Error> {
        let p: InvoicePaidPayload = serde_json::from_value(payload.clone())
            .map_err(|e| anyhow::anyhow!("Failed to parse invoice_paid payload: {}", e))?;

        let paid_at = p.paid_at.unwrap_or_else(Utc::now);
        let days_to_pay = (paid_at.date_naive() - p.created_at.date_naive())
            .num_days()
            .max(0) as i32;

        // (a) Upsert into rpt_payment_history
        sqlx::query(
            r#"
            INSERT INTO rpt_payment_history
                (tenant_id, customer_id, invoice_id, currency,
                 amount_cents, issued_at, paid_at, days_to_pay, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, NOW())
            ON CONFLICT (tenant_id, invoice_id) DO UPDATE SET
                paid_at     = EXCLUDED.paid_at,
                days_to_pay = EXCLUDED.days_to_pay
            "#,
        )
        .bind(tenant_id)
        .bind(&p.customer_id)
        .bind(&p.invoice_id)
        .bind(&p.currency)
        .bind(p.amount_cents)
        .bind(p.created_at)
        .bind(paid_at)
        .bind(days_to_pay)
        .execute(pool)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to upsert payment history: {}", e))?;

        // (b) Transition open invoice cache to 'paid'
        sqlx::query(
            r#"
            UPDATE rpt_open_invoices_cache
            SET status = 'paid', updated_at = NOW()
            WHERE tenant_id = $1 AND invoice_id = $2
            "#,
        )
        .bind(tenant_id)
        .bind(&p.invoice_id)
        .execute(pool)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to update open invoice status: {}", e))?;

        Ok(())
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::ar::invoice_opened::{InvoiceOpenedHandler, SUBJECT_INVOICE_OPENED};
    use crate::ingest::IngestConsumer;
    use event_bus::BusMessage;
    use serial_test::serial;
    use std::sync::Arc;

    const TENANT: &str = "test-inv-paid-tenant";

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
        sqlx::query("DELETE FROM rpt_payment_history WHERE tenant_id = $1")
            .bind(TENANT)
            .execute(pool)
            .await
            .ok();
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

    fn make_opened_envelope(event_id: &str, invoice_id: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "event_id": event_id,
            "tenant_id": TENANT,
            "event_type": "ar.invoice_opened",
            "source_module": "ar",
            "data": {
                "invoice_id": invoice_id,
                "customer_id": "cust-001",
                "app_id": "test-app",
                "amount_cents": 50000,
                "currency": "USD",
                "created_at": "2026-01-15T12:00:00Z",
                "due_at": "2026-02-15T12:00:00Z",
                "paid_at": null
            }
        }))
        .unwrap()
    }

    fn make_paid_envelope(event_id: &str, invoice_id: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "event_id": event_id,
            "tenant_id": TENANT,
            "event_type": "ar.invoice_paid",
            "source_module": "ar",
            "data": {
                "invoice_id": invoice_id,
                "customer_id": "cust-001",
                "app_id": "test-app",
                "amount_cents": 50000,
                "currency": "USD",
                "created_at": "2026-01-15T12:00:00Z",
                "due_at": "2026-02-15T12:00:00Z",
                "paid_at": "2026-02-10T09:30:00Z"
            }
        }))
        .unwrap()
    }

    #[tokio::test]
    #[serial]
    async fn test_invoice_paid_creates_payment_history() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let handler = Arc::new(InvoicePaidHandler);
        let consumer = IngestConsumer::new("test-inv-paid-basic", pool.clone(), handler);

        let msg = BusMessage::new(
            SUBJECT_INVOICE_PAID.to_string(),
            make_paid_envelope("evt-paid-001", "inv-001"),
        );

        let processed = consumer.process_message(&msg).await.expect("process");
        assert!(processed);

        let (amount, days): (i64, i32) = sqlx::query_as(
            "SELECT amount_cents, days_to_pay FROM rpt_payment_history \
             WHERE tenant_id = $1 AND invoice_id = $2",
        )
        .bind(TENANT)
        .bind("inv-001")
        .fetch_one(&pool)
        .await
        .expect("fetch history");

        assert_eq!(amount, 50000);
        assert_eq!(days, 26); // Jan 15 → Feb 10 = 26 days

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_opened_then_paid_transitions_cache() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        // Step 1: invoice_opened
        let opened_handler = Arc::new(InvoiceOpenedHandler);
        let c1 = IngestConsumer::new("test-inv-lifecycle-open", pool.clone(), opened_handler);
        let msg1 = BusMessage::new(
            SUBJECT_INVOICE_OPENED.to_string(),
            make_opened_envelope("evt-lc-open", "inv-lc-001"),
        );
        c1.process_message(&msg1).await.expect("opened");

        let (status_before,): (String,) = sqlx::query_as(
            "SELECT status FROM rpt_open_invoices_cache \
             WHERE tenant_id = $1 AND invoice_id = $2",
        )
        .bind(TENANT)
        .bind("inv-lc-001")
        .fetch_one(&pool)
        .await
        .expect("status before");
        assert_eq!(status_before, "open");

        // Step 2: invoice_paid
        let paid_handler = Arc::new(InvoicePaidHandler);
        let c2 = IngestConsumer::new("test-inv-lifecycle-paid", pool.clone(), paid_handler);
        let msg2 = BusMessage::new(
            SUBJECT_INVOICE_PAID.to_string(),
            make_paid_envelope("evt-lc-paid", "inv-lc-001"),
        );
        c2.process_message(&msg2).await.expect("paid");

        let (status_after,): (String,) = sqlx::query_as(
            "SELECT status FROM rpt_open_invoices_cache \
             WHERE tenant_id = $1 AND invoice_id = $2",
        )
        .bind(TENANT)
        .bind("inv-lc-001")
        .fetch_one(&pool)
        .await
        .expect("status after");
        assert_eq!(status_after, "paid");

        // Payment history should also exist
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM rpt_payment_history \
             WHERE tenant_id = $1 AND invoice_id = $2",
        )
        .bind(TENANT)
        .bind("inv-lc-001")
        .fetch_one(&pool)
        .await
        .expect("history count");
        assert_eq!(count, 1);

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_invoice_paid_replay_safe() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let handler = Arc::new(InvoicePaidHandler);

        // Two different consumers (simulates checkpoint reset replay)
        let c1 = IngestConsumer::new("test-inv-paid-replay-1", pool.clone(), handler.clone());
        let msg1 = BusMessage::new(
            SUBJECT_INVOICE_PAID.to_string(),
            make_paid_envelope("evt-paid-r1", "inv-replay"),
        );
        c1.process_message(&msg1).await.expect("first");

        let c2 = IngestConsumer::new("test-inv-paid-replay-2", pool.clone(), handler);
        let msg2 = BusMessage::new(
            SUBJECT_INVOICE_PAID.to_string(),
            make_paid_envelope("evt-paid-r2", "inv-replay"),
        );
        c2.process_message(&msg2).await.expect("second");

        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM rpt_payment_history \
             WHERE tenant_id = $1 AND invoice_id = $2",
        )
        .bind(TENANT)
        .bind("inv-replay")
        .fetch_one(&pool)
        .await
        .expect("count");

        assert_eq!(count, 1, "must have exactly one history row");

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_invoice_paid_checkpoint_dedup() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let handler = Arc::new(InvoicePaidHandler);
        let consumer = IngestConsumer::new("test-inv-paid-dedup", pool.clone(), handler);

        let msg = BusMessage::new(
            SUBJECT_INVOICE_PAID.to_string(),
            make_paid_envelope("evt-paid-dup", "inv-dup"),
        );

        let first = consumer.process_message(&msg).await.expect("first");
        assert!(first, "first delivery processed");

        let second = consumer.process_message(&msg).await.expect("second");
        assert!(!second, "duplicate must be skipped by checkpoint");

        cleanup(&pool).await;
    }
}
