//! Payments event ingestion for the reporting module.
//!
//! Provides a [`PaymentsHandler`] that subscribes to `payments.events.payment.succeeded`
//! and accumulates cash collection amounts into `rpt_cashflow_cache` (daily granularity).
//!
//! ## Cash flow mapping
//!
//! Each `payment.succeeded` event → `rpt_cashflow_cache` row with:
//!   - `activity_type = 'operating'`
//!   - `line_code = 'cash_collections'`
//!   - `amount_minor` accumulated via `ON CONFLICT DO UPDATE`
//!
//! ## Known limitations (v1)
//!
//! - Payment date uses ingestion timestamp (`Utc::now()`), not the event's
//!   `occurred_at`. This is accurate for real-time processing but may drift
//!   on replays/backfills.
//! - Only `payment.succeeded` is ingested. Failed payments are excluded.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use serde::Deserialize;
use sqlx::PgPool;

use event_bus::EventBus;

use crate::ingest::{start_consumer, IngestConsumer, StreamHandler};

// ── Constants ────────────────────────────────────────────────────────────────

/// NATS subject for payment succeeded events (outbox publisher format).
pub const SUBJECT_PAYMENT_SUCCEEDED: &str = "payments.events.payment.succeeded";

/// Consumer name for cash flow payments ingestion.
pub const CONSUMER_CASHFLOW_PAYMENTS: &str = "reporting.cashflow_payments";

// ── Local payload mirror ─────────────────────────────────────────────────────
//
// Reporting must not depend on the payments crate. We mirror only the fields
// we need from PaymentSucceededPayload.

#[derive(Debug, Deserialize)]
struct PaymentSucceededPayload {
    amount_minor: i64,
    currency: String,
}

// ── Handler ──────────────────────────────────────────────────────────────────

/// Accumulates payment.succeeded events into the cashflow cache.
pub struct PaymentsHandler;

#[async_trait]
impl StreamHandler for PaymentsHandler {
    async fn handle(
        &self,
        pool: &PgPool,
        tenant_id: &str,
        _event_id: &str,
        payload: &serde_json::Value,
    ) -> Result<(), anyhow::Error> {
        let p: PaymentSucceededPayload = serde_json::from_value(payload.clone())
            .map_err(|e| anyhow::anyhow!("Failed to parse payment succeeded payload: {}", e))?;

        // Use current date as the cash flow date (v1 limitation: see module docs).
        let as_of = Utc::now().date_naive();

        sqlx::query(
            r#"
            INSERT INTO rpt_cashflow_cache
                (tenant_id, period_start, period_end, activity_type,
                 line_code, line_label, currency, amount_minor, computed_at)
            VALUES ($1, $2, $2, 'operating', 'cash_collections',
                    'Customer collections', $3, $4, NOW())
            ON CONFLICT (tenant_id, period_start, period_end,
                         activity_type, line_code, currency)
            DO UPDATE SET
                amount_minor = rpt_cashflow_cache.amount_minor + EXCLUDED.amount_minor,
                computed_at  = NOW()
            "#,
        )
        .bind(tenant_id)
        .bind(as_of)
        .bind(&p.currency)
        .bind(p.amount_minor)
        .execute(pool)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to upsert cashflow cache: {}", e))?;

        Ok(())
    }
}

// ── Consumer registration ────────────────────────────────────────────────────

/// Register all Payments ingestion consumers.
///
/// Spawns a background task subscribing to `payments.events.payment.succeeded`
/// and driving [`PaymentsHandler`].
pub fn register_consumers(pool: PgPool, bus: Arc<dyn EventBus>) {
    let handler = Arc::new(PaymentsHandler);
    let consumer = IngestConsumer::new(CONSUMER_CASHFLOW_PAYMENTS, pool, handler);
    start_consumer(consumer, bus, SUBJECT_PAYMENT_SUCCEEDED);
}

// ── Integrated tests (real DB, no mocks) ─────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::IngestConsumer;
    use event_bus::BusMessage;
    use serial_test::serial;

    const TENANT: &str = "test-cf-payments-tenant";

    fn test_db_url() -> String {
        std::env::var("REPORTING_DATABASE_URL")
            .unwrap_or_else(|_| "postgres://ap_user:ap_pass@localhost:5443/reporting_test".into())
    }

    async fn test_pool() -> PgPool {
        let pool = PgPool::connect(&test_db_url()).await.expect("connect");
        sqlx::migrate!("./db/migrations")
            .run(&pool)
            .await
            .expect("migrate");
        pool
    }

    async fn cleanup(pool: &PgPool) {
        sqlx::query("DELETE FROM rpt_cashflow_cache WHERE tenant_id = $1")
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

    fn make_payment_envelope(event_id: &str, amount_minor: i64, currency: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "event_id": event_id,
            "tenant_id": TENANT,
            "event_type": "payment.succeeded",
            "payload": {
                "payment_id": format!("pay-{event_id}"),
                "invoice_id": "inv-001",
                "ar_customer_id": "cust-001",
                "amount_minor": amount_minor,
                "currency": currency
            }
        }))
        .unwrap()
    }

    #[tokio::test]
    #[serial]
    async fn test_payment_succeeded_creates_cashflow_entry() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let handler = Arc::new(PaymentsHandler);
        let consumer = IngestConsumer::new("test-cf-pay-basic", pool.clone(), handler);

        let msg = BusMessage::new(
            SUBJECT_PAYMENT_SUCCEEDED.to_string(),
            make_payment_envelope("evt-cf-001", 259900, "USD"),
        );

        let processed = consumer.process_message(&msg).await.expect("process");
        assert!(processed);

        let (amount,): (i64,) = sqlx::query_as(
            r#"
            SELECT amount_minor FROM rpt_cashflow_cache
            WHERE tenant_id = $1
              AND activity_type = 'operating'
              AND line_code = 'cash_collections'
              AND currency = 'USD'
            "#,
        )
        .bind(TENANT)
        .fetch_one(&pool)
        .await
        .expect("fetch cashflow row");

        assert_eq!(amount, 259900);
        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_multiple_payments_accumulate() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let handler = Arc::new(PaymentsHandler);

        let amounts = [100000_i64, 50000, 75000];
        for (i, &amt) in amounts.iter().enumerate() {
            let consumer = IngestConsumer::new(
                format!("test-cf-pay-accum-{i}"),
                pool.clone(),
                handler.clone(),
            );
            let msg = BusMessage::new(
                SUBJECT_PAYMENT_SUCCEEDED.to_string(),
                make_payment_envelope(&format!("evt-cf-accum-{i}"), amt, "USD"),
            );
            consumer.process_message(&msg).await.expect("process");
        }

        let (total,): (i64,) = sqlx::query_as(
            r#"
            SELECT amount_minor FROM rpt_cashflow_cache
            WHERE tenant_id = $1
              AND activity_type = 'operating'
              AND line_code = 'cash_collections'
              AND currency = 'USD'
            "#,
        )
        .bind(TENANT)
        .fetch_one(&pool)
        .await
        .expect("fetch");

        assert_eq!(total, 225000, "100000 + 50000 + 75000 = 225000");
        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_multi_currency_payments_isolated() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let handler = Arc::new(PaymentsHandler);

        let currencies = [
            ("evt-cf-usd", 100000_i64, "USD"),
            ("evt-cf-eur", 80000, "EUR"),
        ];
        for (i, (eid, amt, cur)) in currencies.iter().enumerate() {
            let consumer = IngestConsumer::new(
                format!("test-cf-pay-cur-{i}"),
                pool.clone(),
                handler.clone(),
            );
            let msg = BusMessage::new(
                SUBJECT_PAYMENT_SUCCEEDED.to_string(),
                make_payment_envelope(eid, *amt, cur),
            );
            consumer.process_message(&msg).await.expect("process");
        }

        let (usd,): (i64,) = sqlx::query_as(
            "SELECT amount_minor FROM rpt_cashflow_cache \
             WHERE tenant_id = $1 AND currency = 'USD' AND line_code = 'cash_collections'",
        )
        .bind(TENANT)
        .fetch_one(&pool)
        .await
        .expect("USD");

        let (eur,): (i64,) = sqlx::query_as(
            "SELECT amount_minor FROM rpt_cashflow_cache \
             WHERE tenant_id = $1 AND currency = 'EUR' AND line_code = 'cash_collections'",
        )
        .bind(TENANT)
        .fetch_one(&pool)
        .await
        .expect("EUR");

        assert_eq!(usd, 100000);
        assert_eq!(eur, 80000);
        cleanup(&pool).await;
    }
}
