//! AP event ingestion for the reporting module.
//!
//! Subscribes to AP domain events and populates `rpt_ap_aging_cache` with
//! aging bucket snapshots per vendor.
//!
//! ## Events consumed
//!
//! - `ap.events.ap.vendor_bill_created`  — add bill to appropriate bucket
//! - `ap.events.ap.vendor_bill_voided`   — subtract voided amount
//! - `ap.events.ap.payment_executed`     — subtract payment amount
//!
//! ## Bucket assignment (bill_created)
//!
//! Relative to `as_of = today`:
//! - **current**: due_date >= as_of
//! - **1-30**: as_of - 30d <= due_date < as_of
//! - **31-60**: as_of - 60d <= due_date < as_of - 30d
//! - **61-90**: as_of - 90d <= due_date < as_of - 60d
//! - **over_90**: due_date < as_of - 90d

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{NaiveDate, Utc};
use serde::Deserialize;
use sqlx::PgPool;

use event_bus::EventBus;

use crate::ingest::{start_consumer, IngestConsumer, StreamHandler};

// ── Constants ────────────────────────────────────────────────────────────────

pub const SUBJECT_BILL_CREATED: &str = "ap.events.ap.vendor_bill_created";
pub const SUBJECT_BILL_VOIDED: &str = "ap.events.ap.vendor_bill_voided";
pub const SUBJECT_PAYMENT_EXECUTED: &str = "ap.events.ap.payment_executed";

pub const CONSUMER_AP_AGING_BILLS: &str = "reporting.ap_aging_bills";
pub const CONSUMER_AP_AGING_VOIDS: &str = "reporting.ap_aging_voids";
pub const CONSUMER_AP_AGING_PAYMENTS: &str = "reporting.ap_aging_payments";

// ── Local payload mirrors ───────────────────────────────────────────────────
//
// Reporting must not depend on the AP crate. Mirror only required fields.

#[derive(Debug, Deserialize)]
struct BillCreatedPayload {
    vendor_id: String,
    total_minor: i64,
    due_date: chrono::DateTime<Utc>,
    currency: String,
}

#[derive(Debug, Deserialize)]
struct BillVoidedPayload {
    vendor_id: String,
    original_total_minor: i64,
    currency: String,
}

#[derive(Debug, Deserialize)]
struct PaymentExecutedPayload {
    vendor_id: String,
    amount_minor: i64,
    currency: String,
}

// ── Bucket computation ──────────────────────────────────────────────────────

enum AgingBucket {
    Current,
    Days1_30,
    Days31_60,
    Days61_90,
    Over90,
}

fn compute_bucket(due_date: NaiveDate, as_of: NaiveDate) -> AgingBucket {
    let days_past_due = (as_of - due_date).num_days();
    if days_past_due <= 0 {
        AgingBucket::Current
    } else if days_past_due <= 30 {
        AgingBucket::Days1_30
    } else if days_past_due <= 60 {
        AgingBucket::Days31_60
    } else if days_past_due <= 90 {
        AgingBucket::Days61_90
    } else {
        AgingBucket::Over90
    }
}

// ── Bill Created Handler ────────────────────────────────────────────────────

pub struct ApBillCreatedHandler;

#[async_trait]
impl StreamHandler for ApBillCreatedHandler {
    async fn handle(
        &self,
        pool: &PgPool,
        tenant_id: &str,
        _event_id: &str,
        payload: &serde_json::Value,
    ) -> Result<(), anyhow::Error> {
        let p: BillCreatedPayload = serde_json::from_value(payload.clone())
            .map_err(|e| anyhow::anyhow!("Failed to parse bill created payload: {}", e))?;

        let as_of = Utc::now().date_naive();
        let due_date = p.due_date.date_naive();

        let (current, b1_30, b31_60, b61_90, over_90) = match compute_bucket(due_date, as_of) {
            AgingBucket::Current => (p.total_minor, 0, 0, 0, 0),
            AgingBucket::Days1_30 => (0, p.total_minor, 0, 0, 0),
            AgingBucket::Days31_60 => (0, 0, p.total_minor, 0, 0),
            AgingBucket::Days61_90 => (0, 0, 0, p.total_minor, 0),
            AgingBucket::Over90 => (0, 0, 0, 0, p.total_minor),
        };

        upsert_aging_add(
            pool, tenant_id, as_of, &p.vendor_id, &p.currency,
            current, b1_30, b31_60, b61_90, over_90, p.total_minor,
        )
        .await
    }
}

// ── Bill Voided Handler ─────────────────────────────────────────────────────

pub struct ApBillVoidedHandler;

#[async_trait]
impl StreamHandler for ApBillVoidedHandler {
    async fn handle(
        &self,
        pool: &PgPool,
        tenant_id: &str,
        _event_id: &str,
        payload: &serde_json::Value,
    ) -> Result<(), anyhow::Error> {
        let p: BillVoidedPayload = serde_json::from_value(payload.clone())
            .map_err(|e| anyhow::anyhow!("Failed to parse bill voided payload: {}", e))?;

        let as_of = Utc::now().date_naive();
        upsert_aging_subtract(pool, tenant_id, as_of, &p.vendor_id, &p.currency, p.original_total_minor)
            .await
    }
}

// ── Payment Executed Handler ────────────────────────────────────────────────

pub struct ApPaymentExecutedHandler;

#[async_trait]
impl StreamHandler for ApPaymentExecutedHandler {
    async fn handle(
        &self,
        pool: &PgPool,
        tenant_id: &str,
        _event_id: &str,
        payload: &serde_json::Value,
    ) -> Result<(), anyhow::Error> {
        let p: PaymentExecutedPayload = serde_json::from_value(payload.clone())
            .map_err(|e| anyhow::anyhow!("Failed to parse payment executed payload: {}", e))?;

        let as_of = Utc::now().date_naive();
        upsert_aging_subtract(pool, tenant_id, as_of, &p.vendor_id, &p.currency, p.amount_minor)
            .await
    }
}

// ── SQL helpers ─────────────────────────────────────────────────────────────

async fn upsert_aging_add(
    pool: &PgPool,
    tenant_id: &str,
    as_of: NaiveDate,
    vendor_id: &str,
    currency: &str,
    current: i64,
    b1_30: i64,
    b31_60: i64,
    b61_90: i64,
    over_90: i64,
    total: i64,
) -> Result<(), anyhow::Error> {
    sqlx::query(
        r#"
        INSERT INTO rpt_ap_aging_cache
            (tenant_id, as_of, vendor_id, currency,
             current_minor, bucket_1_30_minor, bucket_31_60_minor,
             bucket_61_90_minor, bucket_over_90_minor, total_minor,
             computed_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, NOW())
        ON CONFLICT (tenant_id, as_of, vendor_id, currency) DO UPDATE SET
            current_minor        = rpt_ap_aging_cache.current_minor        + EXCLUDED.current_minor,
            bucket_1_30_minor    = rpt_ap_aging_cache.bucket_1_30_minor    + EXCLUDED.bucket_1_30_minor,
            bucket_31_60_minor   = rpt_ap_aging_cache.bucket_31_60_minor   + EXCLUDED.bucket_31_60_minor,
            bucket_61_90_minor   = rpt_ap_aging_cache.bucket_61_90_minor   + EXCLUDED.bucket_61_90_minor,
            bucket_over_90_minor = rpt_ap_aging_cache.bucket_over_90_minor + EXCLUDED.bucket_over_90_minor,
            total_minor          = rpt_ap_aging_cache.total_minor          + EXCLUDED.total_minor,
            computed_at          = NOW()
        "#,
    )
    .bind(tenant_id)
    .bind(as_of)
    .bind(vendor_id)
    .bind(currency)
    .bind(current)
    .bind(b1_30)
    .bind(b31_60)
    .bind(b61_90)
    .bind(over_90)
    .bind(total)
    .execute(pool)
    .await
    .map_err(|e| anyhow::anyhow!("Failed to upsert AP aging cache (add): {}", e))?;
    Ok(())
}

async fn upsert_aging_subtract(
    pool: &PgPool,
    tenant_id: &str,
    as_of: NaiveDate,
    vendor_id: &str,
    currency: &str,
    amount: i64,
) -> Result<(), anyhow::Error> {
    // Reduce amounts (floor at 0 to respect CHECK >= 0 constraints).
    // Best-effort bucket distribution: subtract from current first.
    let result = sqlx::query(
        r#"
        UPDATE rpt_ap_aging_cache SET
            current_minor        = GREATEST(0, current_minor - LEAST(current_minor, $5)),
            total_minor          = GREATEST(0, total_minor - $5),
            computed_at          = NOW()
        WHERE tenant_id = $1
          AND as_of     = $2
          AND vendor_id = $3
          AND currency  = $4
        "#,
    )
    .bind(tenant_id)
    .bind(as_of)
    .bind(vendor_id)
    .bind(currency)
    .bind(amount)
    .execute(pool)
    .await
    .map_err(|e| anyhow::anyhow!("Failed to update AP aging cache (subtract): {}", e))?;

    if result.rows_affected() == 0 {
        tracing::debug!(
            tenant_id, vendor_id, currency,
            "No AP aging row for today to subtract from; skipping"
        );
    }
    Ok(())
}

// ── Consumer registration ────────────────────────────────────────────────────

/// Register all AP aging ingestion consumers.
pub fn register_consumers(pool: PgPool, bus: Arc<dyn EventBus>) {
    let bill_handler = Arc::new(ApBillCreatedHandler);
    let bill_consumer = IngestConsumer::new(CONSUMER_AP_AGING_BILLS, pool.clone(), bill_handler);
    start_consumer(bill_consumer, bus.clone(), SUBJECT_BILL_CREATED);

    let void_handler = Arc::new(ApBillVoidedHandler);
    let void_consumer = IngestConsumer::new(CONSUMER_AP_AGING_VOIDS, pool.clone(), void_handler);
    start_consumer(void_consumer, bus.clone(), SUBJECT_BILL_VOIDED);

    let pay_handler = Arc::new(ApPaymentExecutedHandler);
    let pay_consumer = IngestConsumer::new(CONSUMER_AP_AGING_PAYMENTS, pool, pay_handler);
    start_consumer(pay_consumer, bus, SUBJECT_PAYMENT_EXECUTED);
}

// ── Integrated tests (real DB, no mocks) ─────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::IngestConsumer;
    use event_bus::BusMessage;
    use serial_test::serial;

    const TENANT: &str = "test-ap-aging-tenant";
    const VENDOR_A: &str = "vendor-aaaa-1111";
    const VENDOR_B: &str = "vendor-bbbb-2222";

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
        sqlx::query("DELETE FROM rpt_ap_aging_cache WHERE tenant_id = $1")
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

    fn make_bill_created_envelope(event_id: &str, vendor_id: &str, total_minor: i64, due_date: &str, currency: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "event_id": event_id,
            "tenant_id": TENANT,
            "event_type": "ap.vendor_bill_created",
            "payload": {
                "bill_id": format!("bill-{event_id}"),
                "vendor_id": vendor_id,
                "vendor_invoice_ref": format!("INV-{event_id}"),
                "total_minor": total_minor,
                "due_date": due_date,
                "currency": currency,
                "lines": [],
                "invoice_date": "2026-01-01T00:00:00Z",
                "entered_by": "system",
                "entered_at": "2026-01-01T00:00:00Z"
            }
        }))
        .unwrap()
    }

    fn make_bill_voided_envelope(event_id: &str, vendor_id: &str, original_total: i64, currency: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "event_id": event_id,
            "tenant_id": TENANT,
            "event_type": "ap.vendor_bill_voided",
            "payload": {
                "bill_id": format!("bill-{event_id}"),
                "vendor_id": vendor_id,
                "vendor_invoice_ref": "INV-X",
                "original_total_minor": original_total,
                "currency": currency,
                "void_reason": "test void",
                "voided_by": "system",
                "voided_at": "2026-01-15T00:00:00Z"
            }
        }))
        .unwrap()
    }

    fn make_payment_envelope(event_id: &str, vendor_id: &str, amount: i64, currency: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "event_id": event_id,
            "tenant_id": TENANT,
            "event_type": "ap.payment_executed",
            "payload": {
                "payment_id": format!("pay-{event_id}"),
                "run_id": "run-001",
                "vendor_id": vendor_id,
                "bill_ids": ["bill-001"],
                "amount_minor": amount,
                "currency": currency,
                "payment_method": "ach",
                "executed_at": "2026-01-15T00:00:00Z"
            }
        }))
        .unwrap()
    }

    /// Fetch aging row for a vendor on today's date.
    async fn fetch_aging(pool: &PgPool, vendor_id: &str, currency: &str) -> Option<(i64, i64, i64, i64, i64, i64)> {
        let today = Utc::now().date_naive();
        sqlx::query_as(
            r#"
            SELECT current_minor, bucket_1_30_minor, bucket_31_60_minor,
                   bucket_61_90_minor, bucket_over_90_minor, total_minor
            FROM rpt_ap_aging_cache
            WHERE tenant_id = $1 AND as_of = $2 AND vendor_id = $3 AND currency = $4
            "#,
        )
        .bind(TENANT)
        .bind(today)
        .bind(vendor_id)
        .bind(currency)
        .fetch_optional(pool)
        .await
        .expect("fetch aging")
    }

    #[tokio::test]
    #[serial]
    async fn test_bill_created_populates_current_bucket() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let handler = Arc::new(ApBillCreatedHandler);
        let consumer = IngestConsumer::new("test-ap-aging-cur", pool.clone(), handler);

        // Bill due far in the future → current bucket
        let msg = BusMessage::new(
            SUBJECT_BILL_CREATED.to_string(),
            make_bill_created_envelope("evt-ap-cur-001", VENDOR_A, 50000, "2027-06-15T00:00:00Z", "USD"),
        );
        let processed = consumer.process_message(&msg).await.expect("process");
        assert!(processed);

        let row = fetch_aging(&pool, VENDOR_A, "USD").await.expect("row exists");
        assert_eq!(row.0, 50000, "current_minor");
        assert_eq!(row.5, 50000, "total_minor");
        assert_eq!(row.1, 0, "bucket_1_30 should be 0");

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_bill_created_accumulates_multiple_bills() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let handler = Arc::new(ApBillCreatedHandler);

        // Two bills for the same vendor, both current bucket
        for (i, amt) in [30000_i64, 20000].iter().enumerate() {
            let consumer = IngestConsumer::new(format!("test-ap-aging-acc-{i}"), pool.clone(), handler.clone());
            let msg = BusMessage::new(
                SUBJECT_BILL_CREATED.to_string(),
                make_bill_created_envelope(&format!("evt-ap-acc-{i}"), VENDOR_A, *amt, "2027-06-15T00:00:00Z", "USD"),
            );
            consumer.process_message(&msg).await.expect("process");
        }

        let row = fetch_aging(&pool, VENDOR_A, "USD").await.expect("row");
        assert_eq!(row.0, 50000, "current: 30000 + 20000");
        assert_eq!(row.5, 50000, "total: 30000 + 20000");

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_bill_voided_subtracts_from_aging() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let handler = Arc::new(ApBillCreatedHandler);
        let consumer = IngestConsumer::new("test-ap-aging-void-setup", pool.clone(), handler);

        // Create a bill first
        let msg = BusMessage::new(
            SUBJECT_BILL_CREATED.to_string(),
            make_bill_created_envelope("evt-ap-void-001", VENDOR_A, 80000, "2027-06-15T00:00:00Z", "USD"),
        );
        consumer.process_message(&msg).await.expect("create");

        // Now void part of it
        let void_handler = Arc::new(ApBillVoidedHandler);
        let void_consumer = IngestConsumer::new("test-ap-aging-void-sub", pool.clone(), void_handler);
        let void_msg = BusMessage::new(
            SUBJECT_BILL_VOIDED.to_string(),
            make_bill_voided_envelope("evt-ap-void-002", VENDOR_A, 30000, "USD"),
        );
        void_consumer.process_message(&void_msg).await.expect("void");

        let row = fetch_aging(&pool, VENDOR_A, "USD").await.expect("row");
        assert_eq!(row.0, 50000, "current: 80000 - 30000");
        assert_eq!(row.5, 50000, "total: 80000 - 30000");

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_payment_subtracts_from_aging() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let handler = Arc::new(ApBillCreatedHandler);
        let consumer = IngestConsumer::new("test-ap-aging-pay-setup", pool.clone(), handler);

        let msg = BusMessage::new(
            SUBJECT_BILL_CREATED.to_string(),
            make_bill_created_envelope("evt-ap-pay-001", VENDOR_A, 100000, "2027-06-15T00:00:00Z", "USD"),
        );
        consumer.process_message(&msg).await.expect("create");

        let pay_handler = Arc::new(ApPaymentExecutedHandler);
        let pay_consumer = IngestConsumer::new("test-ap-aging-pay-sub", pool.clone(), pay_handler);
        let pay_msg = BusMessage::new(
            SUBJECT_PAYMENT_EXECUTED.to_string(),
            make_payment_envelope("evt-ap-pay-002", VENDOR_A, 40000, "USD"),
        );
        pay_consumer.process_message(&pay_msg).await.expect("pay");

        let row = fetch_aging(&pool, VENDOR_A, "USD").await.expect("row");
        assert_eq!(row.0, 60000, "current: 100000 - 40000");
        assert_eq!(row.5, 60000, "total: 100000 - 40000");

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_multi_vendor_isolation() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let handler = Arc::new(ApBillCreatedHandler);

        let vendors = [(VENDOR_A, 50000_i64), (VENDOR_B, 30000)];
        for (i, (vid, amt)) in vendors.iter().enumerate() {
            let consumer = IngestConsumer::new(format!("test-ap-aging-mv-{i}"), pool.clone(), handler.clone());
            let msg = BusMessage::new(
                SUBJECT_BILL_CREATED.to_string(),
                make_bill_created_envelope(&format!("evt-ap-mv-{i}"), vid, *amt, "2027-06-15T00:00:00Z", "USD"),
            );
            consumer.process_message(&msg).await.expect("process");
        }

        let a = fetch_aging(&pool, VENDOR_A, "USD").await.expect("vendor A");
        let b = fetch_aging(&pool, VENDOR_B, "USD").await.expect("vendor B");
        assert_eq!(a.5, 50000, "vendor A total");
        assert_eq!(b.5, 30000, "vendor B total");

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_multi_currency_isolation() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let handler = Arc::new(ApBillCreatedHandler);

        let currencies = [("USD", 50000_i64), ("EUR", 30000)];
        for (i, (cur, amt)) in currencies.iter().enumerate() {
            let consumer = IngestConsumer::new(format!("test-ap-aging-mc-{i}"), pool.clone(), handler.clone());
            let msg = BusMessage::new(
                SUBJECT_BILL_CREATED.to_string(),
                make_bill_created_envelope(&format!("evt-ap-mc-{i}"), VENDOR_A, *amt, "2027-06-15T00:00:00Z", cur),
            );
            consumer.process_message(&msg).await.expect("process");
        }

        let usd = fetch_aging(&pool, VENDOR_A, "USD").await.expect("USD");
        let eur = fetch_aging(&pool, VENDOR_A, "EUR").await.expect("EUR");
        assert_eq!(usd.5, 50000, "USD total");
        assert_eq!(eur.5, 30000, "EUR total");

        cleanup(&pool).await;
    }
}
