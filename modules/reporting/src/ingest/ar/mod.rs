//! AR event ingestion for the reporting module.
//!
//! Handlers:
//! - [`ArAgingHandler`]: `ar.events.ar.ar_aging_updated` → `rpt_ar_aging_cache`
//! - [`InvoiceOpenedHandler`]: `ar.events.ar.invoice_opened` → `rpt_open_invoices_cache`
//! - [`InvoicePaidHandler`]: `ar.events.ar.invoice_paid` → `rpt_payment_history` + cache transition
//!
//! ## Idempotency
//!
//! Two layers protect against duplicates:
//! 1. **Framework layer** (`IngestConsumer`): skips events whose `event_id` matches
//!    the checkpoint — covers normal re-delivery.
//! 2. **Handler layer** (`ON CONFLICT DO UPDATE`): upserts replace values on the
//!    cache's unique constraint.

pub mod invoice_opened;
pub mod invoice_paid;

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use sqlx::PgPool;

use event_bus::EventBus;

use crate::ingest::{start_consumer, IngestConsumer, StreamHandler};
use invoice_opened::{InvoiceOpenedHandler, CONSUMER_INVOICE_OPENED, SUBJECT_INVOICE_OPENED};
use invoice_paid::{InvoicePaidHandler, CONSUMER_INVOICE_PAID, SUBJECT_INVOICE_PAID};

// ── Constants ────────────────────────────────────────────────────────────────

/// NATS subject for AR aging updated events.
///
/// The AR publisher maps `event_type = "ar.ar_aging_updated"` to
/// `ar.events.ar.ar_aging_updated`.
pub const SUBJECT_AR_AGING_UPDATED: &str = "ar.events.ar.ar_aging_updated";

/// Consumer name for AR aging cache ingestion.
pub const CONSUMER_AR_AGING: &str = "reporting.ar_aging";

// ── Local payload mirror ─────────────────────────────────────────────────────
//
// Reporting must not depend on the AR crate. We mirror only the fields
// we need from ArAgingUpdatedPayload / AgingBuckets.

#[derive(Debug, Deserialize)]
struct AgingBuckets {
    current_minor: i64,
    days_1_30_minor: i64,
    days_31_60_minor: i64,
    days_61_90_minor: i64,
    days_over_90_minor: i64,
    total_outstanding_minor: i64,
    currency: String,
}

#[derive(Debug, Deserialize)]
struct ArAgingUpdatedPayload {
    #[allow(dead_code)]
    invoice_count: i64,
    buckets: AgingBuckets,
    calculated_at: DateTime<Utc>,
}

// ── Handler ──────────────────────────────────────────────────────────────────

/// Builds the AR aging cache from `ar.ar_aging_updated` events.
pub struct ArAgingHandler;

#[async_trait]
impl StreamHandler for ArAgingHandler {
    async fn handle(
        &self,
        pool: &PgPool,
        tenant_id: &str,
        _event_id: &str,
        payload: &serde_json::Value,
    ) -> Result<(), anyhow::Error> {
        let p: ArAgingUpdatedPayload = serde_json::from_value(payload.clone())
            .map_err(|e| anyhow::anyhow!("Failed to parse AR aging payload: {}", e))?;

        let as_of = p.calculated_at.date_naive();

        // v1: store as tenant-level aggregate since the event doesn't carry customer_id
        let customer_id = "_total";

        sqlx::query(
            r#"
            INSERT INTO rpt_ar_aging_cache
                (tenant_id, as_of, customer_id, currency,
                 current_minor, bucket_1_30_minor, bucket_31_60_minor,
                 bucket_61_90_minor, bucket_over_90_minor, total_minor,
                 computed_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, NOW())
            ON CONFLICT (tenant_id, as_of, customer_id, currency) DO UPDATE SET
                current_minor        = EXCLUDED.current_minor,
                bucket_1_30_minor    = EXCLUDED.bucket_1_30_minor,
                bucket_31_60_minor   = EXCLUDED.bucket_31_60_minor,
                bucket_61_90_minor   = EXCLUDED.bucket_61_90_minor,
                bucket_over_90_minor = EXCLUDED.bucket_over_90_minor,
                total_minor          = EXCLUDED.total_minor,
                computed_at          = NOW()
            "#,
        )
        .bind(tenant_id)
        .bind(as_of)
        .bind(customer_id)
        .bind(&p.buckets.currency)
        .bind(p.buckets.current_minor)
        .bind(p.buckets.days_1_30_minor)
        .bind(p.buckets.days_31_60_minor)
        .bind(p.buckets.days_61_90_minor)
        .bind(p.buckets.days_over_90_minor)
        .bind(p.buckets.total_outstanding_minor)
        .execute(pool)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to upsert AR aging cache: {}", e))?;

        Ok(())
    }
}

// ── Consumer registration ────────────────────────────────────────────────────

/// Register all AR ingestion consumers.
///
/// Spawns background tasks subscribing to:
/// - `ar.events.ar.ar_aging_updated` → aging cache
/// - `ar.events.ar.invoice_opened` → open invoices cache
/// - `ar.events.ar.invoice_paid` → payment history + cache transition
pub fn register_consumers(pool: PgPool, bus: Arc<dyn EventBus>) {
    // Aging
    let aging = Arc::new(ArAgingHandler);
    let aging_consumer = IngestConsumer::new(CONSUMER_AR_AGING, pool.clone(), aging);
    start_consumer(aging_consumer, bus.clone(), SUBJECT_AR_AGING_UPDATED);

    // Invoice opened → rpt_open_invoices_cache
    let opened = Arc::new(InvoiceOpenedHandler);
    let opened_consumer = IngestConsumer::new(CONSUMER_INVOICE_OPENED, pool.clone(), opened);
    start_consumer(opened_consumer, bus.clone(), SUBJECT_INVOICE_OPENED);

    // Invoice paid → rpt_payment_history + cache transition
    let paid = Arc::new(InvoicePaidHandler);
    let paid_consumer = IngestConsumer::new(CONSUMER_INVOICE_PAID, pool, paid);
    start_consumer(paid_consumer, bus, SUBJECT_INVOICE_PAID);
}

// ── Integrated tests (real DB + InMemoryBus, no mocks) ───────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::IngestConsumer;
    use event_bus::BusMessage;
    use serial_test::serial;

    const TENANT: &str = "test-ar-aging-tenant";

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
        sqlx::query("DELETE FROM rpt_ar_aging_cache WHERE tenant_id = $1")
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

    fn make_aging_envelope(
        event_id: &str,
        current: i64,
        d1_30: i64,
        d31_60: i64,
        d61_90: i64,
        over_90: i64,
        currency: &str,
    ) -> Vec<u8> {
        let total = current + d1_30 + d31_60 + d61_90 + over_90;
        serde_json::to_vec(&serde_json::json!({
            "event_id": event_id,
            "tenant_id": TENANT,
            "event_type": "ar.ar_aging_updated",
            "source_module": "ar",
            "payload": {
                "tenant_id": TENANT,
                "invoice_count": 5,
                "buckets": {
                    "current_minor": current,
                    "days_1_30_minor": d1_30,
                    "days_31_60_minor": d31_60,
                    "days_61_90_minor": d61_90,
                    "days_over_90_minor": over_90,
                    "total_outstanding_minor": total,
                    "currency": currency
                },
                "calculated_at": "2026-02-15T12:00:00Z"
            }
        }))
        .unwrap()
    }

    #[tokio::test]
    #[serial]
    async fn test_handle_creates_aging_cache_entry() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let handler = Arc::new(ArAgingHandler);
        let consumer = IngestConsumer::new("test-ar-aging-basic", pool.clone(), handler);

        let msg = BusMessage::new(
            SUBJECT_AR_AGING_UPDATED.to_string(),
            make_aging_envelope("evt-aging-001", 100000, 50000, 20000, 5000, 2000, "USD"),
        );

        let processed = consumer.process_message(&msg).await.expect("process");
        assert!(processed, "first delivery must be processed");

        let (current, b30, b60, b90, over90, total): (i64, i64, i64, i64, i64, i64) =
            sqlx::query_as(
                r#"
                SELECT current_minor, bucket_1_30_minor, bucket_31_60_minor,
                       bucket_61_90_minor, bucket_over_90_minor, total_minor
                FROM rpt_ar_aging_cache
                WHERE tenant_id = $1 AND customer_id = '_total' AND currency = 'USD'
                "#,
            )
            .bind(TENANT)
            .fetch_one(&pool)
            .await
            .expect("fetch aging row");

        assert_eq!(current, 100000);
        assert_eq!(b30, 50000);
        assert_eq!(b60, 20000);
        assert_eq!(b90, 5000);
        assert_eq!(over90, 2000);
        assert_eq!(total, 177000);

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_handle_replaces_on_conflict() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let handler = Arc::new(ArAgingHandler);

        // First event
        let consumer1 = IngestConsumer::new("test-ar-aging-replace-1", pool.clone(), handler.clone());
        let msg1 = BusMessage::new(
            SUBJECT_AR_AGING_UPDATED.to_string(),
            make_aging_envelope("evt-aging-r1", 100000, 50000, 0, 0, 0, "USD"),
        );
        consumer1.process_message(&msg1).await.expect("first");

        // Second event (same date/currency) — should REPLACE, not accumulate
        let consumer2 = IngestConsumer::new("test-ar-aging-replace-2", pool.clone(), handler);
        let msg2 = BusMessage::new(
            SUBJECT_AR_AGING_UPDATED.to_string(),
            make_aging_envelope("evt-aging-r2", 80000, 30000, 10000, 0, 0, "USD"),
        );
        consumer2.process_message(&msg2).await.expect("second");

        let (current, b30, total): (i64, i64, i64) = sqlx::query_as(
            r#"
            SELECT current_minor, bucket_1_30_minor, total_minor
            FROM rpt_ar_aging_cache
            WHERE tenant_id = $1 AND customer_id = '_total' AND currency = 'USD'
            "#,
        )
        .bind(TENANT)
        .fetch_one(&pool)
        .await
        .expect("fetch");

        assert_eq!(current, 80000, "current must be replaced, not accumulated");
        assert_eq!(b30, 30000, "1-30 bucket must be replaced");
        assert_eq!(total, 120000, "total must reflect replaced values");

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_idempotent_on_redelivery() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let handler = Arc::new(ArAgingHandler);
        let consumer = IngestConsumer::new("test-ar-aging-idem", pool.clone(), handler);

        let msg = BusMessage::new(
            SUBJECT_AR_AGING_UPDATED.to_string(),
            make_aging_envelope("evt-aging-idem", 100000, 0, 0, 0, 0, "USD"),
        );

        let first = consumer.process_message(&msg).await.expect("first");
        assert!(first, "first delivery processed");

        let second = consumer.process_message(&msg).await.expect("second");
        assert!(!second, "re-delivery must be skipped by checkpoint");

        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM rpt_ar_aging_cache WHERE tenant_id = $1 AND currency = 'USD'",
        )
        .bind(TENANT)
        .fetch_one(&pool)
        .await
        .expect("count");

        assert_eq!(count, 1, "must have exactly one cache row");

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_multi_currency_aging_isolated() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let handler = Arc::new(ArAgingHandler);

        let currencies = [
            ("evt-aging-usd", 100000_i64, "USD"),
            ("evt-aging-eur", 80000, "EUR"),
        ];

        for (i, (eid, current, cur)) in currencies.iter().enumerate() {
            let consumer = IngestConsumer::new(
                format!("test-ar-aging-cur-{i}"),
                pool.clone(),
                handler.clone(),
            );
            let msg = BusMessage::new(
                SUBJECT_AR_AGING_UPDATED.to_string(),
                make_aging_envelope(eid, *current, 0, 0, 0, 0, cur),
            );
            consumer.process_message(&msg).await.expect("process");
        }

        let (usd_total,): (i64,) = sqlx::query_as(
            "SELECT total_minor FROM rpt_ar_aging_cache \
             WHERE tenant_id = $1 AND currency = 'USD'",
        )
        .bind(TENANT)
        .fetch_one(&pool)
        .await
        .expect("USD");

        let (eur_total,): (i64,) = sqlx::query_as(
            "SELECT total_minor FROM rpt_ar_aging_cache \
             WHERE tenant_id = $1 AND currency = 'EUR'",
        )
        .bind(TENANT)
        .fetch_one(&pool)
        .await
        .expect("EUR");

        assert_eq!(usd_total, 100000);
        assert_eq!(eur_total, 80000);

        cleanup(&pool).await;
    }
}
