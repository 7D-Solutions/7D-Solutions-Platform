/// Integrated E2E: AR/AP aging caches reconcile to module totals (bd-2cvz)
///
/// Verifies the full pipeline from event ingestion into the reporting cache
/// and asserts that:
///   1. Reporting AR aging (from rpt_ar_aging_cache) matches manually ingested totals
///   2. Reporting AP aging (from rpt_ap_aging_cache) matches manually ingested totals
///   3. Cross-tenant contamination does not occur
///
/// Strategy:
/// - Insert AR aging events via ArAgingHandler → assert rpt_ar_aging_cache totals
/// - Insert AP bill/payment events via ApBillCreatedHandler/ApPaymentExecutedHandler
///   → assert rpt_ap_aging_cache totals
/// - Two tenants used throughout to verify isolation
///
/// Run with: cargo test -p e2e-tests -- reporting_aging --nocapture
mod common;

use chrono::Utc;
use common::get_reporting_pool;
use event_bus::BusMessage;
use reporting::domain::aging::{ap_aging, ar_aging};
use reporting::ingest::ap::{
    ApBillCreatedHandler, ApPaymentExecutedHandler, SUBJECT_BILL_CREATED, SUBJECT_PAYMENT_EXECUTED,
};
use reporting::ingest::ar::{ArAgingHandler, SUBJECT_AR_AGING_UPDATED};
use reporting::ingest::IngestConsumer;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

// ── Setup / teardown ─────────────────────────────────────────────────────────

async fn setup_reporting_pool() -> PgPool {
    let pool = get_reporting_pool().await;
    sqlx::migrate!("../modules/reporting/db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run reporting migrations");
    pool
}

async fn cleanup_tenant(pool: &PgPool, tenant_id: &str) {
    for table in &[
        "rpt_ar_aging_cache",
        "rpt_ap_aging_cache",
        "rpt_ingestion_checkpoints",
    ] {
        sqlx::query(&format!("DELETE FROM {} WHERE tenant_id = $1", table))
            .bind(tenant_id)
            .execute(pool)
            .await
            .ok();
    }
}

// ── AR event envelope builder ─────────────────────────────────────────────────

/// Build an `ar.ar_aging_updated` EventEnvelope (matches ArAgingUpdatedPayload mirror).
fn make_ar_aging_envelope(
    tenant_id: &str,
    event_id: &str,
    currency: &str,
    current: i64,
    b30: i64,
    b60: i64,
    b90: i64,
    over90: i64,
) -> Vec<u8> {
    let total = current + b30 + b60 + b90 + over90;
    serde_json::to_vec(&serde_json::json!({
        "event_id": event_id,
        "tenant_id": tenant_id,
        "data": {
            "invoice_count": 1,
            "buckets": {
                "current_minor": current,
                "days_1_30_minor": b30,
                "days_31_60_minor": b60,
                "days_61_90_minor": b90,
                "days_over_90_minor": over90,
                "total_outstanding_minor": total,
                "currency": currency
            },
            "calculated_at": Utc::now().to_rfc3339()
        }
    }))
    .unwrap()
}

// ── AP event envelope builders ────────────────────────────────────────────────

fn make_ap_bill_envelope(
    tenant_id: &str,
    event_id: &str,
    vendor_id: &str,
    total_minor: i64,
    currency: &str,
    due_offset_days: i64,
) -> Vec<u8> {
    let due_date = Utc::now() + chrono::Duration::days(due_offset_days);
    serde_json::to_vec(&serde_json::json!({
        "event_id": event_id,
        "tenant_id": tenant_id,
        "payload": {
            "vendor_id": vendor_id,
            "total_minor": total_minor,
            "due_date": due_date.to_rfc3339(),
            "currency": currency
        }
    }))
    .unwrap()
}

fn make_ap_payment_envelope(
    tenant_id: &str,
    event_id: &str,
    vendor_id: &str,
    amount_minor: i64,
    currency: &str,
) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "event_id": event_id,
        "tenant_id": tenant_id,
        "payload": {
            "vendor_id": vendor_id,
            "amount_minor": amount_minor,
            "currency": currency
        }
    }))
    .unwrap()
}

// ── Test 1: AR aging cache reconciles to ingested totals ──────────────────────

/// Ingests an AR aging event with known bucket totals and asserts the reporting
/// cache returns the same totals for the same as_of date.
#[tokio::test]
async fn test_ar_aging_cache_reconciles_to_ingested_totals() {
    let pool = setup_reporting_pool().await;
    let tenant_id = format!("e2e-ar-aging-{}", Uuid::new_v4());
    cleanup_tenant(&pool, &tenant_id).await;

    let handler = Arc::new(ArAgingHandler);
    let consumer_name = format!("e2e-ar-aging-{}", &tenant_id[..8]);

    // Ingest one AR aging snapshot: $10,000 current, $5,000 1-30 days
    let msg = BusMessage::new(
        SUBJECT_AR_AGING_UPDATED.to_string(),
        make_ar_aging_envelope(
            &tenant_id,
            &Uuid::new_v4().to_string(),
            "USD",
            1000000,
            500000,
            0,
            0,
            0,
        ),
    );
    let consumer = IngestConsumer::new(consumer_name.clone(), pool.clone(), handler);
    let processed = consumer
        .process_message(&msg)
        .await
        .expect("AR aging ingest failed");
    assert!(processed, "AR aging event must be processed");

    // Query the reporting cache for today
    let today = Utc::now().date_naive();
    let summary = ar_aging::get_aging_summary(&pool, &tenant_id, today)
        .await
        .expect("AR aging query failed");

    assert!(
        !summary.is_empty(),
        "AR aging cache must have rows after ingest"
    );
    let usd = summary
        .iter()
        .find(|s| s.currency == "USD")
        .expect("USD bucket");
    assert_eq!(usd.current_minor, 1000000, "current bucket mismatch");
    assert_eq!(usd.bucket_1_30_minor, 500000, "1-30 bucket mismatch");
    assert_eq!(usd.total_minor, 1500000, "total_minor mismatch");

    println!(
        "PASS: AR aging cache reconciles — total={}",
        usd.total_minor
    );
    cleanup_tenant(&pool, &tenant_id).await;
}

// ── Test 2: AP aging cache reconciles to ingested bills/payments ──────────────

/// Creates AP bill events, ingests them, then verifies the reporting AP aging
/// cache totals match the expected values after a partial payment.
#[tokio::test]
async fn test_ap_aging_cache_reconciles_to_ingested_bills_and_payments() {
    let pool = setup_reporting_pool().await;
    let tenant_id = format!("e2e-ap-aging-{}", Uuid::new_v4());
    cleanup_tenant(&pool, &tenant_id).await;

    let vendor_id = format!("vendor-{}", Uuid::new_v4());
    let bill_handler = Arc::new(ApBillCreatedHandler);
    let pay_handler = Arc::new(ApPaymentExecutedHandler);

    // Bill 1: $8,000 USD due in 5 days → current bucket
    let bill1_id = Uuid::new_v4().to_string();
    let c1 = IngestConsumer::new(
        format!("e2e-ap-bill1-{}", &tenant_id[..8]),
        pool.clone(),
        bill_handler.clone(),
    );
    c1.process_message(&BusMessage::new(
        SUBJECT_BILL_CREATED.to_string(),
        make_ap_bill_envelope(&tenant_id, &bill1_id, &vendor_id, 800000, "USD", 5),
    ))
    .await
    .expect("bill1 ingest failed");

    // Bill 2: $3,000 USD due in 10 days → current bucket
    let bill2_id = Uuid::new_v4().to_string();
    let c2 = IngestConsumer::new(
        format!("e2e-ap-bill2-{}", &tenant_id[..8]),
        pool.clone(),
        bill_handler.clone(),
    );
    c2.process_message(&BusMessage::new(
        SUBJECT_BILL_CREATED.to_string(),
        make_ap_bill_envelope(&tenant_id, &bill2_id, &vendor_id, 300000, "USD", 10),
    ))
    .await
    .expect("bill2 ingest failed");

    // Partial payment: $4,000
    let pay_id = Uuid::new_v4().to_string();
    let c3 = IngestConsumer::new(
        format!("e2e-ap-pay-{}", &tenant_id[..8]),
        pool.clone(),
        pay_handler,
    );
    c3.process_message(&BusMessage::new(
        SUBJECT_PAYMENT_EXECUTED.to_string(),
        make_ap_payment_envelope(&tenant_id, &pay_id, &vendor_id, 400000, "USD"),
    ))
    .await
    .expect("payment ingest failed");

    // Query reporting AP aging cache
    let today = Utc::now().date_naive();
    let report = ap_aging::query_ap_aging(&pool, &tenant_id, today)
        .await
        .expect("AP aging query failed");

    assert!(
        !report.vendors.is_empty(),
        "AP aging cache must have vendor rows"
    );

    let summary = report
        .summary_by_currency
        .iter()
        .find(|s| s.currency == "USD")
        .expect("USD summary");
    // 800000 + 300000 - 400000 = 700000 remaining
    assert_eq!(
        summary.total_minor, 700000,
        "AP aging total after partial payment"
    );

    println!(
        "PASS: AP aging cache reconciles — total={}",
        summary.total_minor
    );
    cleanup_tenant(&pool, &tenant_id).await;
}

// ── Test 3: Cross-tenant isolation ───────────────────────────────────────────

/// Ingests AR aging events for two tenants and asserts that each tenant's
/// reporting cache contains only its own data (no cross-tenant contamination).
#[tokio::test]
async fn test_reporting_aging_no_cross_tenant_contamination() {
    let pool = setup_reporting_pool().await;
    let tenant_a = format!("e2e-iso-a-{}", Uuid::new_v4());
    let tenant_b = format!("e2e-iso-b-{}", Uuid::new_v4());
    cleanup_tenant(&pool, &tenant_a).await;
    cleanup_tenant(&pool, &tenant_b).await;

    let handler = Arc::new(ArAgingHandler);

    // Tenant A: $50,000 current
    let ca = IngestConsumer::new(
        format!("e2e-iso-ar-a-{}", &tenant_a[..8]),
        pool.clone(),
        handler.clone(),
    );
    ca.process_message(&BusMessage::new(
        SUBJECT_AR_AGING_UPDATED.to_string(),
        make_ar_aging_envelope(
            &tenant_a,
            &Uuid::new_v4().to_string(),
            "USD",
            5000000,
            0,
            0,
            0,
            0,
        ),
    ))
    .await
    .expect("tenant A ingest failed");

    // Tenant B: $20,000 current
    let cb = IngestConsumer::new(
        format!("e2e-iso-ar-b-{}", &tenant_b[..8]),
        pool.clone(),
        handler.clone(),
    );
    cb.process_message(&BusMessage::new(
        SUBJECT_AR_AGING_UPDATED.to_string(),
        make_ar_aging_envelope(
            &tenant_b,
            &Uuid::new_v4().to_string(),
            "USD",
            2000000,
            0,
            0,
            0,
            0,
        ),
    ))
    .await
    .expect("tenant B ingest failed");

    let today = Utc::now().date_naive();

    let summary_a = ar_aging::get_aging_summary(&pool, &tenant_a, today)
        .await
        .expect("tenant A query failed");
    let summary_b = ar_aging::get_aging_summary(&pool, &tenant_b, today)
        .await
        .expect("tenant B query failed");

    let a_usd = summary_a
        .iter()
        .find(|s| s.currency == "USD")
        .expect("tenant A USD");
    let b_usd = summary_b
        .iter()
        .find(|s| s.currency == "USD")
        .expect("tenant B USD");

    assert_eq!(a_usd.current_minor, 5000000, "tenant A current mismatch");
    assert_eq!(b_usd.current_minor, 2000000, "tenant B current mismatch");
    assert_ne!(
        a_usd.current_minor, b_usd.current_minor,
        "tenants must not share data"
    );

    println!(
        "PASS: Cross-tenant isolation — tenant_a={}, tenant_b={}",
        a_usd.current_minor, b_usd.current_minor
    );
    cleanup_tenant(&pool, &tenant_a).await;
    cleanup_tenant(&pool, &tenant_b).await;
}

// ── Test 4: AP aging AP cross-tenant isolation ────────────────────────────────

/// Ingests AP bill events for two tenants and asserts AP cache isolation.
#[tokio::test]
async fn test_ap_aging_no_cross_tenant_contamination() {
    let pool = setup_reporting_pool().await;
    let tenant_a = format!("e2e-ap-iso-a-{}", Uuid::new_v4());
    let tenant_b = format!("e2e-ap-iso-b-{}", Uuid::new_v4());
    cleanup_tenant(&pool, &tenant_a).await;
    cleanup_tenant(&pool, &tenant_b).await;

    let handler = Arc::new(ApBillCreatedHandler);
    let vendor = "shared-vendor-ref";

    // Tenant A: $12,000 bill
    IngestConsumer::new(
        format!("e2e-ap-iso-bill-a-{}", &tenant_a[..8]),
        pool.clone(),
        handler.clone(),
    )
    .process_message(&BusMessage::new(
        SUBJECT_BILL_CREATED.to_string(),
        make_ap_bill_envelope(
            &tenant_a,
            &Uuid::new_v4().to_string(),
            vendor,
            1200000,
            "USD",
            7,
        ),
    ))
    .await
    .expect("tenant A AP bill failed");

    // Tenant B: $5,000 bill
    IngestConsumer::new(
        format!("e2e-ap-iso-bill-b-{}", &tenant_b[..8]),
        pool.clone(),
        handler.clone(),
    )
    .process_message(&BusMessage::new(
        SUBJECT_BILL_CREATED.to_string(),
        make_ap_bill_envelope(
            &tenant_b,
            &Uuid::new_v4().to_string(),
            vendor,
            500000,
            "USD",
            7,
        ),
    ))
    .await
    .expect("tenant B AP bill failed");

    let today = Utc::now().date_naive();
    let report_a = ap_aging::query_ap_aging(&pool, &tenant_a, today)
        .await
        .expect("A query");
    let report_b = ap_aging::query_ap_aging(&pool, &tenant_b, today)
        .await
        .expect("B query");

    let a_total = report_a
        .summary_by_currency
        .iter()
        .find(|s| s.currency == "USD")
        .map(|s| s.total_minor)
        .unwrap_or(0);
    let b_total = report_b
        .summary_by_currency
        .iter()
        .find(|s| s.currency == "USD")
        .map(|s| s.total_minor)
        .unwrap_or(0);

    assert_eq!(a_total, 1200000, "tenant A AP total mismatch");
    assert_eq!(b_total, 500000, "tenant B AP total mismatch");

    println!(
        "PASS: AP cross-tenant isolation — A={}, B={}",
        a_total, b_total
    );
    cleanup_tenant(&pool, &tenant_a).await;
    cleanup_tenant(&pool, &tenant_b).await;
}
