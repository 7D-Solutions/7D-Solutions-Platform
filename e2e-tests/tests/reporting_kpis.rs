/// Integrated E2E: KPI endpoint sanity checks (bd-2bvi)
///
/// Validates the unified KPI endpoint returns consistent, non-negative, and
/// reconcilable values after seeding AR/AP/inventory data into reporting caches.
///
/// Scenario:
///   1. Ingest AR aging snapshot → rpt_ar_aging_cache
///   2. Ingest AP bill + partial payment → rpt_ap_aging_cache
///   3. Ingest inventory valuation snapshot → rpt_kpi_cache
///   4. Call compute_kpis and assert values match expectations
///   5. Re-call to verify determinism
///
/// Run with: cargo test -p e2e-tests -- reporting_kpis --nocapture
mod common;

use chrono::Utc;
use common::get_reporting_pool;
use event_bus::BusMessage;
use reporting::domain::kpis::compute_kpis;
use reporting::ingest::ap::{
    ApBillCreatedHandler, ApPaymentExecutedHandler, SUBJECT_BILL_CREATED, SUBJECT_PAYMENT_EXECUTED,
};
use reporting::ingest::ar::{ArAgingHandler, SUBJECT_AR_AGING_UPDATED};
use reporting::ingest::inventory::{InventoryValueHandler, SUBJECT_VALUATION_SNAPSHOT};
use reporting::ingest::IngestConsumer;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

// ── Helpers ───────────────────────────────────────────────────────────────────

async fn setup_reporting_pool() -> PgPool {
    let pool = get_reporting_pool().await;
    sqlx::migrate!("../modules/reporting/db/migrations")
        .run(&pool)
        .await
        .expect("migrations failed");
    pool
}

async fn cleanup_tenant(pool: &PgPool, tenant_id: &str) {
    for table in &[
        "rpt_ar_aging_cache",
        "rpt_ap_aging_cache",
        "rpt_kpi_cache",
        "rpt_cashflow_cache",
        "rpt_trial_balance_cache",
        "rpt_ingestion_checkpoints",
    ] {
        sqlx::query(&format!("DELETE FROM {} WHERE tenant_id = $1", table))
            .bind(tenant_id)
            .execute(pool)
            .await
            .ok();
    }
}

fn make_ar_aging_envelope(
    tenant_id: &str,
    event_id: &str,
    currency: &str,
    current: i64,
    b30: i64,
) -> Vec<u8> {
    let total = current + b30;
    serde_json::to_vec(&serde_json::json!({
        "event_id": event_id,
        "tenant_id": tenant_id,
        "data": {
            "invoice_count": 1,
            "buckets": {
                "current_minor": current,
                "days_1_30_minor": b30,
                "days_31_60_minor": 0,
                "days_61_90_minor": 0,
                "days_over_90_minor": 0,
                "total_outstanding_minor": total,
                "currency": currency
            },
            "calculated_at": Utc::now().to_rfc3339()
        }
    }))
    .unwrap()
}

fn make_ap_bill_envelope(
    tenant_id: &str,
    event_id: &str,
    vendor_id: &str,
    total: i64,
    currency: &str,
) -> Vec<u8> {
    let due = Utc::now() + chrono::Duration::days(15);
    serde_json::to_vec(&serde_json::json!({
        "event_id": event_id,
        "tenant_id": tenant_id,
        "payload": {
            "vendor_id": vendor_id,
            "total_minor": total,
            "due_date": due.to_rfc3339(),
            "currency": currency
        }
    }))
    .unwrap()
}

fn make_ap_payment_envelope(
    tenant_id: &str,
    event_id: &str,
    vendor_id: &str,
    amount: i64,
    currency: &str,
) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "event_id": event_id,
        "tenant_id": tenant_id,
        "payload": {
            "vendor_id": vendor_id,
            "amount_minor": amount,
            "currency": currency
        }
    }))
    .unwrap()
}

fn make_inventory_envelope(
    tenant_id: &str,
    event_id: &str,
    as_of: &str,
    currency: &str,
    value: i64,
) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "event_id": event_id,
        "tenant_id": tenant_id,
        "payload": {
            "as_of": as_of,
            "currency": currency,
            "total_value_minor": value
        }
    }))
    .unwrap()
}

// ── Test 1: Full KPI snapshot reconciles to seeded data ───────────────────────

#[tokio::test]
async fn test_kpi_snapshot_reconciles_to_ingested_data() {
    let pool = setup_reporting_pool().await;
    let tenant_id = format!("e2e-kpi-{}", Uuid::new_v4());
    cleanup_tenant(&pool, &tenant_id).await;

    let vendor_id = format!("v-{}", Uuid::new_v4());
    let today = Utc::now().date_naive();
    let today_str = today.to_string(); // "2026-MM-DD"

    // ── AR: $15,000 current + $5,000 1-30 = $20,000 total ────────────────────
    IngestConsumer::new(
        format!("e2e-kpi-ar-{}", &tenant_id[..8]),
        pool.clone(),
        Arc::new(ArAgingHandler),
    )
    .process_message(&BusMessage::new(
        SUBJECT_AR_AGING_UPDATED.to_string(),
        make_ar_aging_envelope(
            &tenant_id,
            &Uuid::new_v4().to_string(),
            "USD",
            1500000,
            500000,
        ),
    ))
    .await
    .expect("AR ingest");

    // ── AP: $12,000 bill, $2,000 paid → $10,000 outstanding ──────────────────
    IngestConsumer::new(
        format!("e2e-kpi-apb-{}", &tenant_id[..8]),
        pool.clone(),
        Arc::new(ApBillCreatedHandler),
    )
    .process_message(&BusMessage::new(
        SUBJECT_BILL_CREATED.to_string(),
        make_ap_bill_envelope(
            &tenant_id,
            &Uuid::new_v4().to_string(),
            &vendor_id,
            1200000,
            "USD",
        ),
    ))
    .await
    .expect("AP bill ingest");

    IngestConsumer::new(
        format!("e2e-kpi-app-{}", &tenant_id[..8]),
        pool.clone(),
        Arc::new(ApPaymentExecutedHandler),
    )
    .process_message(&BusMessage::new(
        SUBJECT_PAYMENT_EXECUTED.to_string(),
        make_ap_payment_envelope(
            &tenant_id,
            &Uuid::new_v4().to_string(),
            &vendor_id,
            200000,
            "USD",
        ),
    ))
    .await
    .expect("AP payment ingest");

    // ── Inventory: $5,000 value ───────────────────────────────────────────────
    IngestConsumer::new(
        format!("e2e-kpi-inv-{}", &tenant_id[..8]),
        pool.clone(),
        Arc::new(InventoryValueHandler),
    )
    .process_message(&BusMessage::new(
        SUBJECT_VALUATION_SNAPSHOT.to_string(),
        make_inventory_envelope(
            &tenant_id,
            &Uuid::new_v4().to_string(),
            &today_str,
            "USD",
            500000,
        ),
    ))
    .await
    .expect("inventory ingest");

    // ── Query KPI snapshot ────────────────────────────────────────────────────
    let kpis = compute_kpis(&pool, &tenant_id, today)
        .await
        .expect("compute_kpis");

    // AR: $20,000
    let ar_usd = kpis.ar_total_outstanding.get("USD").copied().unwrap_or(0);
    assert_eq!(ar_usd, 2000000, "AR outstanding mismatch");

    // AP: $10,000 (12k - 2k payment)
    let ap_usd = kpis.ap_total_outstanding.get("USD").copied().unwrap_or(0);
    assert_eq!(ap_usd, 1000000, "AP outstanding mismatch");

    // Inventory: $5,000
    let inv_usd = kpis.inventory_value.get("USD").copied().unwrap_or(0);
    assert_eq!(inv_usd, 500000, "Inventory value mismatch");

    // All values non-negative
    for (k, v) in &kpis.ar_total_outstanding {
        assert!(*v >= 0, "AR {} is negative: {}", k, v);
    }
    for (k, v) in &kpis.ap_total_outstanding {
        assert!(*v >= 0, "AP {} is negative: {}", k, v);
    }

    println!(
        "PASS: KPI snapshot — AR={}, AP={}, inv={}",
        ar_usd, ap_usd, inv_usd
    );
    cleanup_tenant(&pool, &tenant_id).await;
}

// ── Test 2: KPI snapshot is deterministic ─────────────────────────────────────

#[tokio::test]
async fn test_kpi_snapshot_is_deterministic() {
    let pool = setup_reporting_pool().await;
    let tenant_id = format!("e2e-kpi-det-{}", Uuid::new_v4());
    cleanup_tenant(&pool, &tenant_id).await;

    let today = Utc::now().date_naive();
    let today_str = today.to_string();

    // Seed minimal data
    IngestConsumer::new(
        format!("e2e-kpi-det-ar-{}", &tenant_id[..8]),
        pool.clone(),
        Arc::new(ArAgingHandler),
    )
    .process_message(&BusMessage::new(
        SUBJECT_AR_AGING_UPDATED.to_string(),
        make_ar_aging_envelope(
            &tenant_id,
            &Uuid::new_v4().to_string(),
            "USD",
            800000,
            200000,
        ),
    ))
    .await
    .expect("AR ingest");

    IngestConsumer::new(
        format!("e2e-kpi-det-inv-{}", &tenant_id[..8]),
        pool.clone(),
        Arc::new(InventoryValueHandler),
    )
    .process_message(&BusMessage::new(
        SUBJECT_VALUATION_SNAPSHOT.to_string(),
        make_inventory_envelope(
            &tenant_id,
            &Uuid::new_v4().to_string(),
            &today_str,
            "USD",
            300000,
        ),
    ))
    .await
    .expect("inventory ingest");

    // Query twice — must be identical
    let kpis1 = compute_kpis(&pool, &tenant_id, today).await.expect("kpis1");
    let kpis2 = compute_kpis(&pool, &tenant_id, today).await.expect("kpis2");

    assert_eq!(
        kpis1.ar_total_outstanding, kpis2.ar_total_outstanding,
        "AR not deterministic"
    );
    assert_eq!(
        kpis1.ap_total_outstanding, kpis2.ap_total_outstanding,
        "AP not deterministic"
    );
    assert_eq!(
        kpis1.inventory_value, kpis2.inventory_value,
        "inventory not deterministic"
    );
    assert_eq!(kpis1.mrr, kpis2.mrr, "MRR not deterministic");
    assert_eq!(
        kpis1.cash_collected_ytd, kpis2.cash_collected_ytd,
        "cash not deterministic"
    );

    println!("PASS: KPI snapshot is deterministic across calls");
    cleanup_tenant(&pool, &tenant_id).await;
}

// ── Test 3: Multi-currency KPI isolation ─────────────────────────────────────

#[tokio::test]
async fn test_kpi_multi_currency_isolation() {
    let pool = setup_reporting_pool().await;
    let tenant_id = format!("e2e-kpi-mc-{}", Uuid::new_v4());
    cleanup_tenant(&pool, &tenant_id).await;

    let today = Utc::now().date_naive();
    let today_str = today.to_string();
    let handler = Arc::new(ArAgingHandler);

    // Ingest USD and EUR AR aging
    for (i, (cur, cur_minor, b30)) in [("USD", 600000_i64, 200000_i64), ("EUR", 300000, 100000)]
        .iter()
        .enumerate()
    {
        let total = cur_minor + b30;
        let msg_bytes = serde_json::to_vec(&serde_json::json!({
            "event_id": format!("evt-kpi-mc-{}-{}", i, &tenant_id[..8]),
            "tenant_id": tenant_id,
            "data": {
                "invoice_count": 2,
                "buckets": {
                    "current_minor": cur_minor,
                    "days_1_30_minor": b30,
                    "days_31_60_minor": 0,
                    "days_61_90_minor": 0,
                    "days_over_90_minor": 0,
                    "total_outstanding_minor": total,
                    "currency": cur
                },
                "calculated_at": Utc::now().to_rfc3339()
            }
        }))
        .unwrap();
        IngestConsumer::new(
            format!("e2e-kpi-mc-ar-{}-{}", cur, &tenant_id[..8]),
            pool.clone(),
            handler.clone(),
        )
        .process_message(&BusMessage::new(
            SUBJECT_AR_AGING_UPDATED.to_string(),
            msg_bytes,
        ))
        .await
        .expect("AR multi-cur ingest");
    }

    // Ingest inventory in USD and EUR
    for (cur, val) in [("USD", 400000_i64), ("EUR", 250000)] {
        IngestConsumer::new(
            format!("e2e-kpi-mc-inv-{}-{}", cur, &tenant_id[..8]),
            pool.clone(),
            Arc::new(InventoryValueHandler),
        )
        .process_message(&BusMessage::new(
            SUBJECT_VALUATION_SNAPSHOT.to_string(),
            make_inventory_envelope(
                &tenant_id,
                &format!("evt-kpi-mc-inv-{}-{}", cur, &tenant_id[..8]),
                &today_str,
                cur,
                val,
            ),
        ))
        .await
        .expect("inventory multi-cur ingest");
    }

    let kpis = compute_kpis(&pool, &tenant_id, today).await.expect("kpis");

    assert_eq!(
        kpis.ar_total_outstanding.get("USD").copied().unwrap_or(0),
        800000,
        "USD AR"
    );
    assert_eq!(
        kpis.ar_total_outstanding.get("EUR").copied().unwrap_or(0),
        400000,
        "EUR AR"
    );
    assert_eq!(
        kpis.inventory_value.get("USD").copied().unwrap_or(0),
        400000,
        "USD inventory"
    );
    assert_eq!(
        kpis.inventory_value.get("EUR").copied().unwrap_or(0),
        250000,
        "EUR inventory"
    );

    println!("PASS: Multi-currency KPI isolation verified");
    cleanup_tenant(&pool, &tenant_id).await;
}
