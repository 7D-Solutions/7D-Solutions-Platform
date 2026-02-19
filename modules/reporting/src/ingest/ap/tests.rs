//! Integrated tests for AP aging ingestion handlers.
//!
//! Tests run against a real Postgres database (reporting_test).

use std::sync::Arc;

use chrono::{Duration, Utc};
use serial_test::serial;
use sqlx::PgPool;

use event_bus::BusMessage;

use crate::ingest::IngestConsumer;

use super::*;

// ── Helpers ──────────────────────────────────────────────────────────────────

const TENANT: &str = "test-ap-aging-ingest";

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
    // Also clear checkpoints so tests are re-runnable with the same event IDs.
    sqlx::query("DELETE FROM rpt_ingestion_checkpoints WHERE consumer_name LIKE 'test-ap-%'")
        .execute(pool)
        .await
        .ok();
}

async fn fetch_aging(pool: &PgPool, vendor_id: &str, currency: &str) -> Option<(i64, i64, i64, i64, i64, i64)> {
    sqlx::query_as::<_, (i64, i64, i64, i64, i64, i64)>(
        r#"
        SELECT current_minor, bucket_1_30_minor, bucket_31_60_minor,
               bucket_61_90_minor, bucket_over_90_minor, total_minor
        FROM rpt_ap_aging_cache
        WHERE tenant_id = $1 AND vendor_id = $2 AND currency = $3
        ORDER BY as_of DESC
        LIMIT 1
        "#,
    )
    .bind(TENANT)
    .bind(vendor_id)
    .bind(currency)
    .fetch_optional(pool)
    .await
    .expect("fetch")
}

// ── Envelope builders ────────────────────────────────────────────────────────

fn make_bill_created_envelope(
    event_id: &str,
    vendor_id: &str,
    total_minor: i64,
    currency: &str,
    due_date: chrono::DateTime<Utc>,
) -> Vec<u8> {
    let envelope = serde_json::json!({
        "event_id": event_id,
        "tenant_id": TENANT,
        "event_type": "ap.vendor_bill_created",
        "timestamp": Utc::now().to_rfc3339(),
        "payload": {
            "vendor_id": vendor_id,
            "total_minor": total_minor,
            "due_date": due_date.to_rfc3339(),
            "currency": currency,
        }
    });
    serde_json::to_vec(&envelope).expect("serialize")
}

fn make_bill_voided_envelope(
    event_id: &str,
    vendor_id: &str,
    original_total_minor: i64,
    currency: &str,
) -> Vec<u8> {
    let envelope = serde_json::json!({
        "event_id": event_id,
        "tenant_id": TENANT,
        "event_type": "ap.vendor_bill_voided",
        "timestamp": Utc::now().to_rfc3339(),
        "payload": {
            "vendor_id": vendor_id,
            "original_total_minor": original_total_minor,
            "currency": currency,
        }
    });
    serde_json::to_vec(&envelope).expect("serialize")
}

fn make_payment_envelope(
    event_id: &str,
    vendor_id: &str,
    amount_minor: i64,
    currency: &str,
) -> Vec<u8> {
    let envelope = serde_json::json!({
        "event_id": event_id,
        "tenant_id": TENANT,
        "event_type": "ap.payment_executed",
        "timestamp": Utc::now().to_rfc3339(),
        "payload": {
            "vendor_id": vendor_id,
            "amount_minor": amount_minor,
            "currency": currency,
        }
    });
    serde_json::to_vec(&envelope).expect("serialize")
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn test_bill_created_populates_current_bucket() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    let handler = Arc::new(ApBillCreatedHandler);
    let consumer = IngestConsumer::new("test-ap-bill-current", pool.clone(), handler);

    // Due date in the future → current bucket
    let due = Utc::now() + Duration::days(10);
    let msg = BusMessage::new(
        SUBJECT_BILL_CREATED.to_string(),
        make_bill_created_envelope("evt-ap-bill-1", "v-100", 50000, "USD", due),
    );
    consumer.process_message(&msg).await.expect("process");

    let row = fetch_aging(&pool, "v-100", "USD").await.expect("row");
    assert_eq!(row.0, 50000, "current_minor");
    assert_eq!(row.1, 0, "bucket_1_30");
    assert_eq!(row.5, 50000, "total_minor");

    cleanup(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_bill_created_accumulates_multiple_bills() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    let handler = Arc::new(ApBillCreatedHandler);

    // Two bills for same vendor, both current bucket
    let due = Utc::now() + Duration::days(5);
    for (i, amt) in [30000_i64, 20000].iter().enumerate() {
        let consumer = IngestConsumer::new(
            format!("test-ap-bill-accum-{i}"),
            pool.clone(),
            handler.clone(),
        );
        let msg = BusMessage::new(
            SUBJECT_BILL_CREATED.to_string(),
            make_bill_created_envelope(&format!("evt-ap-accum-{i}"), "v-200", *amt, "USD", due),
        );
        consumer.process_message(&msg).await.expect("process");
    }

    let row = fetch_aging(&pool, "v-200", "USD").await.expect("row");
    assert_eq!(row.0, 50000, "current_minor = 30000 + 20000");
    assert_eq!(row.5, 50000, "total_minor = 30000 + 20000");

    cleanup(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_bill_voided_subtracts_from_aging() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    let handler_bill = Arc::new(ApBillCreatedHandler);
    let handler_void = Arc::new(ApBillVoidedHandler);

    // Create a bill
    let due = Utc::now() + Duration::days(15);
    let consumer1 = IngestConsumer::new("test-ap-void-1", pool.clone(), handler_bill);
    let msg1 = BusMessage::new(
        SUBJECT_BILL_CREATED.to_string(),
        make_bill_created_envelope("evt-ap-void-bill", "v-300", 80000, "USD", due),
    );
    consumer1.process_message(&msg1).await.expect("process bill");

    // Void it
    let consumer2 = IngestConsumer::new("test-ap-void-2", pool.clone(), handler_void);
    let msg2 = BusMessage::new(
        SUBJECT_BILL_VOIDED.to_string(),
        make_bill_voided_envelope("evt-ap-void-void", "v-300", 80000, "USD"),
    );
    consumer2.process_message(&msg2).await.expect("process void");

    let row = fetch_aging(&pool, "v-300", "USD").await.expect("row");
    assert_eq!(row.0, 0, "current_minor after void");
    assert_eq!(row.5, 0, "total_minor after void");

    cleanup(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_payment_subtracts_from_aging() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    let handler_bill = Arc::new(ApBillCreatedHandler);
    let handler_pay = Arc::new(ApPaymentExecutedHandler);

    // Create a bill
    let due = Utc::now() + Duration::days(10);
    let consumer1 = IngestConsumer::new("test-ap-pay-1", pool.clone(), handler_bill);
    let msg1 = BusMessage::new(
        SUBJECT_BILL_CREATED.to_string(),
        make_bill_created_envelope("evt-ap-pay-bill", "v-400", 100000, "USD", due),
    );
    consumer1.process_message(&msg1).await.expect("process bill");

    // Partial payment
    let consumer2 = IngestConsumer::new("test-ap-pay-2", pool.clone(), handler_pay);
    let msg2 = BusMessage::new(
        SUBJECT_PAYMENT_EXECUTED.to_string(),
        make_payment_envelope("evt-ap-pay-pay", "v-400", 40000, "USD"),
    );
    consumer2.process_message(&msg2).await.expect("process payment");

    let row = fetch_aging(&pool, "v-400", "USD").await.expect("row");
    assert_eq!(row.0, 60000, "current_minor after partial payment");
    assert_eq!(row.5, 60000, "total_minor after partial payment");

    cleanup(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_multi_vendor_isolation() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    let handler = Arc::new(ApBillCreatedHandler);
    let due = Utc::now() + Duration::days(5);

    let vendors = [("v-500", 25000_i64), ("v-501", 75000)];
    for (i, (vid, amt)) in vendors.iter().enumerate() {
        let consumer = IngestConsumer::new(
            format!("test-ap-multi-vendor-{i}"),
            pool.clone(),
            handler.clone(),
        );
        let msg = BusMessage::new(
            SUBJECT_BILL_CREATED.to_string(),
            make_bill_created_envelope(&format!("evt-ap-mv-{i}"), vid, *amt, "USD", due),
        );
        consumer.process_message(&msg).await.expect("process");
    }

    let r1 = fetch_aging(&pool, "v-500", "USD").await.expect("v-500");
    assert_eq!(r1.5, 25000, "v-500 total");

    let r2 = fetch_aging(&pool, "v-501", "USD").await.expect("v-501");
    assert_eq!(r2.5, 75000, "v-501 total");

    cleanup(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_multi_currency_isolation() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    let handler = Arc::new(ApBillCreatedHandler);
    let due = Utc::now() + Duration::days(5);

    let currencies = [("USD", 60000_i64), ("EUR", 40000)];
    for (i, (cur, amt)) in currencies.iter().enumerate() {
        let consumer = IngestConsumer::new(
            format!("test-ap-multi-cur-{i}"),
            pool.clone(),
            handler.clone(),
        );
        let msg = BusMessage::new(
            SUBJECT_BILL_CREATED.to_string(),
            make_bill_created_envelope(&format!("evt-ap-mc-{i}"), "v-600", *amt, cur, due),
        );
        consumer.process_message(&msg).await.expect("process");
    }

    let usd = fetch_aging(&pool, "v-600", "USD").await.expect("USD");
    assert_eq!(usd.5, 60000, "USD total");

    let eur = fetch_aging(&pool, "v-600", "EUR").await.expect("EUR");
    assert_eq!(eur.5, 40000, "EUR total");

    cleanup(&pool).await;
}
