/// Integration test: trace-to-invoice equivalence.
///
/// Validates that invoices generated from metering include line items matching
/// the TTP price application trace, and that a stable trace_hash linkage exists
/// from the billing run item back to the metering trace inputs.
///
/// Requires:
///   - DATABASE_URL pointing at a running TTP Postgres instance
///   - AR_BASE_URL pointing at a running AR service
///   - TENANT_REGISTRY_URL pointing at a running tenant-registry service
///
/// Run with: cargo test -p ttp-rs --test billing_metering_integration -- --ignored
use chrono::{TimeZone, Utc};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use ttp_rs::clients::ar::ArClient;
use ttp_rs::clients::tenant_registry::TenantRegistryClient;
use ttp_rs::domain::billing::{compute_trace_hash, run_billing};
use ttp_rs::domain::metering::{compute_price_trace, ingest_events, MeteringEventInput};

/// Connect to the TTP test database and run migrations.
async fn test_pool() -> PgPool {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5450/ttp_db".to_string());
    let pool = PgPool::connect(&url).await.expect("connect TTP test db");

    sqlx::migrate!("./db/migrations")
        .run(&pool)
        .await
        .expect("run migrations");

    pool
}

/// Clean up all test data for a specific tenant.
async fn cleanup(pool: &PgPool, tenant_id: Uuid) {
    // Order matters: items before runs (FK constraint)
    sqlx::query(
        "DELETE FROM ttp_billing_run_items WHERE run_id IN \
         (SELECT run_id FROM ttp_billing_runs WHERE tenant_id = $1)",
    )
    .bind(tenant_id)
    .execute(pool)
    .await
    .ok();
    sqlx::query("DELETE FROM ttp_billing_runs WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM ttp_one_time_charges WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM ttp_service_agreements WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM ttp_customers WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM ttp_metering_events WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM ttp_metering_pricing WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
}

/// Seed pricing rules for the test tenant.
async fn seed_pricing(pool: &PgPool, tenant_id: Uuid) {
    sqlx::query(
        r#"INSERT INTO ttp_metering_pricing
           (tenant_id, dimension, unit_price_minor, currency, effective_from)
           VALUES ($1, 'api_calls', 10, 'usd', '2026-01-01'),
                  ($1, 'storage_gb', 500, 'usd', '2026-01-01')"#,
    )
    .bind(tenant_id)
    .execute(pool)
    .await
    .expect("seed pricing");
}

/// Build fixed metering events for Feb 2026.
fn feb_events(tenant_id: Uuid) -> Vec<MeteringEventInput> {
    vec![
        MeteringEventInput {
            tenant_id,
            dimension: "api_calls".to_string(),
            quantity: 100,
            occurred_at: Utc.with_ymd_and_hms(2026, 2, 5, 10, 0, 0).unwrap(),
            idempotency_key: "bmi-evt-001".to_string(),
            source_ref: Some("test-harness".to_string()),
        },
        MeteringEventInput {
            tenant_id,
            dimension: "api_calls".to_string(),
            quantity: 50,
            occurred_at: Utc.with_ymd_and_hms(2026, 2, 10, 14, 30, 0).unwrap(),
            idempotency_key: "bmi-evt-002".to_string(),
            source_ref: None,
        },
        MeteringEventInput {
            tenant_id,
            dimension: "storage_gb".to_string(),
            quantity: 5,
            occurred_at: Utc.with_ymd_and_hms(2026, 2, 1, 0, 0, 0).unwrap(),
            idempotency_key: "bmi-evt-003".to_string(),
            source_ref: None,
        },
    ]
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Full E2E: ingest metering events → run billing → verify trace-to-invoice linkage.
///
/// This test validates that:
///   1. The billing run creates a billing_run_item for metered usage
///   2. The item amount matches the price trace total exactly
///   3. The trace_hash stored in the item matches an independently computed hash
#[tokio::test]
#[ignore]
async fn trace_to_invoice_equivalence() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    cleanup(&pool, tenant_id).await;

    // 1. Seed metering data
    seed_pricing(&pool, tenant_id).await;
    let events = feb_events(tenant_id);
    ingest_events(&pool, &events).await.expect("ingest events");

    // 2. Compute expected trace independently
    let trace = compute_price_trace(&pool, tenant_id, "2026-02")
        .await
        .expect("compute trace");

    // Expected: api_calls=150 @ 10 = 1500, storage_gb=5 @ 500 = 2500, total=4000
    assert_eq!(trace.total_minor, 4000, "expected trace total");
    assert_eq!(trace.line_items.len(), 2, "two dimensions");

    let expected_hash = compute_trace_hash(&trace);

    // 3. Run billing (requires AR + tenant-registry services)
    let registry_url = std::env::var("TENANT_REGISTRY_URL")
        .unwrap_or_else(|_| "http://localhost:8092".to_string());
    let ar_url =
        std::env::var("AR_BASE_URL").unwrap_or_else(|_| "http://localhost:8086".to_string());

    let registry = TenantRegistryClient::new(registry_url);
    let ar = ArClient::new(ar_url);

    let summary = run_billing(&pool, &registry, &ar, tenant_id, "2026-02", "bmi-key-001")
        .await
        .expect("billing run");

    assert!(!summary.was_noop, "first run must not be a no-op");

    // 4. Query the billing_run_item for the metering entry (party_id = tenant_id)
    let metering_item = sqlx::query(
        r#"SELECT amount_minor, currency, trace_hash, status
           FROM ttp_billing_run_items
           WHERE run_id = $1 AND party_id = $2"#,
    )
    .bind(summary.run_id)
    .bind(tenant_id)
    .fetch_optional(&pool)
    .await
    .expect("query metering billing item");

    let item = metering_item.expect("metering billing run item must exist");
    let amount: i64 = item.try_get("amount_minor").expect("amount_minor");
    let currency: String = item.try_get("currency").expect("currency");
    let trace_hash: Option<String> = item.try_get("trace_hash").expect("trace_hash");
    let status: String = item.try_get("status").expect("status");

    // 5. Verify trace-to-invoice equivalence
    assert_eq!(
        amount, trace.total_minor,
        "invoice amount must match trace total exactly"
    );
    assert_eq!(currency, trace.currency, "currency must match");
    assert_eq!(status, "invoiced", "item must be invoiced");

    let stored_hash = trace_hash.expect("trace_hash must be present for metering items");
    assert_eq!(
        stored_hash, expected_hash,
        "stored trace_hash must match independently computed hash"
    );

    // 6. Verify idempotency: second billing run is a no-op
    let summary2 = run_billing(&pool, &registry, &ar, tenant_id, "2026-02", "bmi-key-001")
        .await
        .expect("second billing run");

    assert!(summary2.was_noop, "second run must be a no-op");
    assert_eq!(summary2.run_id, summary.run_id, "same run_id");

    cleanup(&pool, tenant_id).await;
}

/// Verify that non-metering billing run items have NULL trace_hash.
#[tokio::test]
#[ignore]
async fn non_metering_items_have_null_trace_hash() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    let party_id = Uuid::new_v4();
    cleanup(&pool, tenant_id).await;

    // Seed only a service agreement (no metering events)
    sqlx::query(
        "INSERT INTO ttp_customers (tenant_id, party_id, status) \
         VALUES ($1, $2, 'active') ON CONFLICT DO NOTHING",
    )
    .bind(tenant_id)
    .bind(party_id)
    .execute(&pool)
    .await
    .expect("seed customer");

    sqlx::query(
        r#"INSERT INTO ttp_service_agreements
           (tenant_id, party_id, plan_code, amount_minor, currency, effective_from)
           VALUES ($1, $2, 'basic', 10000, 'usd', '2026-01-01')"#,
    )
    .bind(tenant_id)
    .bind(party_id)
    .execute(&pool)
    .await
    .expect("seed agreement");

    let registry_url = std::env::var("TENANT_REGISTRY_URL")
        .unwrap_or_else(|_| "http://localhost:8092".to_string());
    let ar_url =
        std::env::var("AR_BASE_URL").unwrap_or_else(|_| "http://localhost:8086".to_string());

    let registry = TenantRegistryClient::new(registry_url);
    let ar = ArClient::new(ar_url);

    let summary = run_billing(&pool, &registry, &ar, tenant_id, "2026-02", "nmi-key-001")
        .await
        .expect("billing run");

    // The agreement item should have NULL trace_hash
    let item = sqlx::query(
        r#"SELECT trace_hash FROM ttp_billing_run_items
           WHERE run_id = $1 AND party_id = $2"#,
    )
    .bind(summary.run_id)
    .bind(party_id)
    .fetch_one(&pool)
    .await
    .expect("query agreement billing item");

    let trace_hash: Option<String> = item.try_get("trace_hash").expect("trace_hash");
    assert!(
        trace_hash.is_none(),
        "non-metering items must have NULL trace_hash"
    );

    cleanup(&pool, tenant_id).await;
}
