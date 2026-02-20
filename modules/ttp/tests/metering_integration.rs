/// Integration tests for TTP metering: idempotent ingestion + deterministic trace.
///
/// Requires DATABASE_URL pointing at a running TTP Postgres instance.
/// Run with: cargo test -p ttp-rs --test metering_integration -- --ignored

use chrono::{TimeZone, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use ttp_rs::domain::metering::{
    compute_price_trace, ingest_event, ingest_events, MeteringEventInput,
};

/// Connect to the TTP test database.
async fn test_pool() -> PgPool {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5450/ttp_db".to_string());
    let pool = PgPool::connect(&url).await.expect("connect TTP test db");

    // Run migrations
    sqlx::migrate!("./db/migrations")
        .run(&pool)
        .await
        .expect("run migrations");

    pool
}

/// Clean up test data for a specific tenant.
async fn cleanup(pool: &PgPool, tenant_id: Uuid) {
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

/// Seed pricing rules for test tenant.
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

/// Build a fixed set of test events.
fn test_events(tenant_id: Uuid) -> Vec<MeteringEventInput> {
    vec![
        MeteringEventInput {
            tenant_id,
            dimension: "api_calls".to_string(),
            quantity: 100,
            occurred_at: Utc.with_ymd_and_hms(2026, 2, 5, 10, 0, 0).unwrap(),
            idempotency_key: "evt-001".to_string(),
            source_ref: Some("test-harness".to_string()),
        },
        MeteringEventInput {
            tenant_id,
            dimension: "api_calls".to_string(),
            quantity: 50,
            occurred_at: Utc.with_ymd_and_hms(2026, 2, 10, 14, 30, 0).unwrap(),
            idempotency_key: "evt-002".to_string(),
            source_ref: None,
        },
        MeteringEventInput {
            tenant_id,
            dimension: "storage_gb".to_string(),
            quantity: 5,
            occurred_at: Utc.with_ymd_and_hms(2026, 2, 1, 0, 0, 0).unwrap(),
            idempotency_key: "evt-003".to_string(),
            source_ref: None,
        },
        MeteringEventInput {
            tenant_id,
            dimension: "api_calls".to_string(),
            quantity: 25,
            occurred_at: Utc.with_ymd_and_hms(2026, 2, 20, 8, 0, 0).unwrap(),
            idempotency_key: "evt-004".to_string(),
            source_ref: Some("batch-job".to_string()),
        },
    ]
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn ingestion_is_idempotent() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    cleanup(&pool, tenant_id).await;

    let event = MeteringEventInput {
        tenant_id,
        dimension: "api_calls".to_string(),
        quantity: 42,
        occurred_at: Utc.with_ymd_and_hms(2026, 2, 15, 12, 0, 0).unwrap(),
        idempotency_key: "idem-test-001".to_string(),
        source_ref: None,
    };

    // First insert — should succeed
    let r1 = ingest_event(&pool, &event).await.expect("first ingest");
    assert!(!r1.was_duplicate, "first insert must not be duplicate");
    assert!(r1.event_id.is_some(), "first insert must return event_id");

    // Second insert — same idempotency_key, must be a no-op
    let r2 = ingest_event(&pool, &event).await.expect("second ingest");
    assert!(r2.was_duplicate, "second insert must be duplicate");
    assert!(r2.event_id.is_none(), "duplicate must not return event_id");

    // Only one row in DB
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ttp_metering_events WHERE tenant_id = $1 AND idempotency_key = $2",
    )
    .bind(tenant_id)
    .bind("idem-test-001")
    .fetch_one(&pool)
    .await
    .expect("count");

    assert_eq!(count, 1, "exactly one row despite two ingestion attempts");

    cleanup(&pool, tenant_id).await;
}

#[tokio::test]
#[ignore]
async fn batch_ingestion_handles_duplicates() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    cleanup(&pool, tenant_id).await;

    let events = test_events(tenant_id);

    // First batch
    let r1 = ingest_events(&pool, &events).await.expect("first batch");
    let ingested1: usize = r1.iter().filter(|r| !r.was_duplicate).count();
    assert_eq!(ingested1, 4, "all 4 events should be new");

    // Second batch — all should be duplicates
    let r2 = ingest_events(&pool, &events).await.expect("second batch");
    let dupes: usize = r2.iter().filter(|r| r.was_duplicate).count();
    assert_eq!(dupes, 4, "all 4 events should be duplicates on re-send");

    cleanup(&pool, tenant_id).await;
}

#[tokio::test]
#[ignore]
async fn price_trace_is_deterministic() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    cleanup(&pool, tenant_id).await;
    seed_pricing(&pool, tenant_id).await;

    let events = test_events(tenant_id);
    ingest_events(&pool, &events).await.expect("ingest events");

    // Compute trace twice — must be identical
    let trace1 = compute_price_trace(&pool, tenant_id, "2026-02")
        .await
        .expect("trace run 1");
    let trace2 = compute_price_trace(&pool, tenant_id, "2026-02")
        .await
        .expect("trace run 2");

    // Same number of line items
    assert_eq!(
        trace1.line_items.len(),
        trace2.line_items.len(),
        "line item count must match"
    );

    // Same total
    assert_eq!(trace1.total_minor, trace2.total_minor, "totals must match");

    // Verify specific values:
    // api_calls: 100 + 50 + 25 = 175, at 10 minor/unit = 1750
    // storage_gb: 5, at 500 minor/unit = 2500
    // total: 4250
    assert_eq!(trace1.total_minor, 4250, "total must be 4250 minor units");
    assert_eq!(trace1.line_items.len(), 2, "two dimensions");

    let api = trace1
        .line_items
        .iter()
        .find(|li| li.dimension == "api_calls")
        .expect("api_calls line item");
    assert_eq!(api.total_quantity, 175);
    assert_eq!(api.event_count, 3);
    assert_eq!(api.unit_price_minor, 10);
    assert_eq!(api.line_total_minor, 1750);

    let storage = trace1
        .line_items
        .iter()
        .find(|li| li.dimension == "storage_gb")
        .expect("storage_gb line item");
    assert_eq!(storage.total_quantity, 5);
    assert_eq!(storage.event_count, 1);
    assert_eq!(storage.unit_price_minor, 500);
    assert_eq!(storage.line_total_minor, 2500);

    // Verify each line item matches across runs
    for (li1, li2) in trace1.line_items.iter().zip(trace2.line_items.iter()) {
        assert_eq!(li1.dimension, li2.dimension);
        assert_eq!(li1.total_quantity, li2.total_quantity);
        assert_eq!(li1.event_count, li2.event_count);
        assert_eq!(li1.unit_price_minor, li2.unit_price_minor);
        assert_eq!(li1.line_total_minor, li2.line_total_minor);
    }

    cleanup(&pool, tenant_id).await;
}

#[tokio::test]
#[ignore]
async fn trace_empty_period_returns_zero() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    cleanup(&pool, tenant_id).await;
    seed_pricing(&pool, tenant_id).await;

    // No events ingested — trace should return empty
    let trace = compute_price_trace(&pool, tenant_id, "2026-02")
        .await
        .expect("trace empty period");

    assert_eq!(trace.line_items.len(), 0, "no line items for empty period");
    assert_eq!(trace.total_minor, 0, "total must be zero");
}

#[tokio::test]
#[ignore]
async fn events_outside_period_excluded() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    cleanup(&pool, tenant_id).await;
    seed_pricing(&pool, tenant_id).await;

    // Ingest events in Feb AND March
    let events = vec![
        MeteringEventInput {
            tenant_id,
            dimension: "api_calls".to_string(),
            quantity: 100,
            occurred_at: Utc.with_ymd_and_hms(2026, 2, 15, 10, 0, 0).unwrap(),
            idempotency_key: "feb-evt".to_string(),
            source_ref: None,
        },
        MeteringEventInput {
            tenant_id,
            dimension: "api_calls".to_string(),
            quantity: 200,
            occurred_at: Utc.with_ymd_and_hms(2026, 3, 5, 10, 0, 0).unwrap(),
            idempotency_key: "mar-evt".to_string(),
            source_ref: None,
        },
    ];

    ingest_events(&pool, &events).await.expect("ingest");

    // Feb trace should only include Feb events
    let trace = compute_price_trace(&pool, tenant_id, "2026-02")
        .await
        .expect("feb trace");

    assert_eq!(trace.line_items.len(), 1);
    let api = &trace.line_items[0];
    assert_eq!(api.total_quantity, 100, "only Feb event counted");
    assert_eq!(api.line_total_minor, 1000); // 100 * 10

    cleanup(&pool, tenant_id).await;
}
