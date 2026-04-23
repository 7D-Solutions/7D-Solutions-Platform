use std::time::Duration;

use chrono::{TimeZone, Utc};
use integrations_rs::domain::sync::{
    compute_comparable_hash, compute_fingerprint, observations, truncate_to_millis,
};
use serde_json::json;
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use tokio::sync::OnceCell;
use uuid::Uuid;

static TEST_POOL: OnceCell<sqlx::PgPool> = OnceCell::const_new();

async fn init_pool() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://integrations_user:integrations_pass@localhost:5449/integrations_db".to_string()
    });
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(Duration::from_secs(90))
        .connect(&url)
        .await
        .expect("Failed to connect to integrations test DB");
    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run integrations migrations");
    pool
}

async fn setup_db() -> sqlx::PgPool {
    TEST_POOL.get_or_init(init_pool).await.clone()
}

fn unique_app() -> String {
    format!("obs-test-{}", Uuid::new_v4().simple())
}

async fn cleanup(pool: &sqlx::PgPool, app_id: &str) {
    sqlx::query("DELETE FROM integrations_sync_observations WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
}

fn millis_ts(ms: i64) -> chrono::DateTime<Utc> {
    Utc.timestamp_millis_opt(ms).single().unwrap()
}

// ── Dedupe (pure) tests ───────────────────────────────────────────────────────

#[test]
fn fingerprint_prefers_sync_token_over_timestamp() {
    let ts = millis_ts(1_700_000_000_123);
    let payload = json!({"id": 1});
    let fp = compute_fingerprint(Some("st-abc"), Some(ts), &payload);
    assert_eq!(fp, "st:st-abc");
}

#[test]
fn fingerprint_falls_back_to_timestamp_ms() {
    let ts = millis_ts(1_700_000_000_999);
    let payload = json!({"id": 1});
    let fp = compute_fingerprint(None, Some(ts), &payload);
    assert_eq!(fp, "ts:1700000000999");
}

#[test]
fn fingerprint_falls_back_to_payload_hash_when_both_absent() {
    let payload = json!({"a": 1, "b": 2});
    let fp = compute_fingerprint(None, None, &payload);
    assert!(fp.starts_with("ph:"), "got: {fp}");
    // sha256 hex is 64 chars; prefix is 3
    assert_eq!(fp.len(), 67);
}

#[test]
fn fingerprint_payload_hash_is_key_order_independent() {
    let a = json!({"x": 10, "y": 20});
    let b = json!({"y": 20, "x": 10});
    assert_eq!(
        compute_fingerprint(None, None, &a),
        compute_fingerprint(None, None, &b),
        "key order must not affect ph: fingerprint"
    );
}

#[test]
fn comparable_hash_is_sub_millisecond_stable() {
    // Two nanosecond-precision timestamps that share the same millisecond must
    // produce the same comparable_hash.
    let fields = json!({"amount": 999});
    let ts_a = Utc.timestamp_nanos(1_700_000_000_123_000_000_i64);
    let ts_b = Utc.timestamp_nanos(1_700_000_000_123_999_999_i64);
    assert_ne!(ts_a, ts_b);

    let h_a = compute_comparable_hash(&fields, ts_a);
    let h_b = compute_comparable_hash(&fields, ts_b);
    assert_eq!(h_a, h_b, "same millisecond → same comparable_hash");
}

#[test]
fn truncate_to_millis_drops_sub_ms_precision() {
    let precise = Utc.timestamp_nanos(1_700_000_000_123_456_789_i64);
    let trunc = truncate_to_millis(precise);
    assert_eq!(trunc.timestamp_millis(), precise.timestamp_millis());
    assert_eq!(
        trunc.timestamp_subsec_micros() % 1000,
        0,
        "sub-ms must be zero"
    );
}

// ── DB integration tests ──────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn test_upsert_observation_inserts_new_row() {
    let pool = setup_db().await;
    let app_id = unique_app();
    cleanup(&pool, &app_id).await;

    let ts = truncate_to_millis(millis_ts(1_700_000_000_000));
    let payload = json!({"id": "cust-001", "name": "Acme"});
    let fp = compute_fingerprint(Some("st-v1"), None, &payload);
    let ch = compute_comparable_hash(&payload, ts);

    let row = observations::upsert_observation(
        &pool,
        &app_id,
        "quickbooks",
        "customer",
        "cust-001",
        &fp,
        ts,
        &ch,
        1,
        &payload,
        "test",
        false,
    )
    .await
    .expect("upsert");

    assert_eq!(row.app_id, app_id);
    assert_eq!(row.fingerprint, fp);
    assert_eq!(row.comparable_hash, ch);
    assert_eq!(row.projection_version, 1);
    assert_eq!(row.last_updated_time, ts);

    cleanup(&pool, &app_id).await;
}

#[tokio::test]
#[serial]
async fn test_upsert_deduplicates_on_unique_key() {
    let pool = setup_db().await;
    let app_id = unique_app();
    cleanup(&pool, &app_id).await;

    let ts = truncate_to_millis(millis_ts(1_700_000_001_000));
    let payload = json!({"id": "cust-002", "balance": 100});
    let fp = compute_fingerprint(Some("st-v2"), None, &payload);
    let ch = compute_comparable_hash(&payload, ts);

    // First insert
    let r1 = observations::upsert_observation(
        &pool,
        &app_id,
        "quickbooks",
        "customer",
        "cust-002",
        &fp,
        ts,
        &ch,
        1,
        &payload,
        "test",
        false,
    )
    .await
    .expect("first upsert");

    // Second upsert with same key — same payload, should return the existing row
    let r2 = observations::upsert_observation(
        &pool,
        &app_id,
        "quickbooks",
        "customer",
        "cust-002",
        &fp,
        ts,
        &ch,
        1,
        &payload,
        "test",
        false,
    )
    .await
    .expect("second upsert");

    assert_eq!(r1.id, r2.id, "same fingerprint must return same row");

    // Only one row should exist
    let count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM integrations_sync_observations WHERE app_id = $1")
            .bind(&app_id)
            .fetch_one(&pool)
            .await
            .expect("count");

    assert_eq!(count.0, 1, "deduplication must produce exactly one row");

    cleanup(&pool, &app_id).await;
}

#[tokio::test]
#[serial]
async fn test_different_fingerprints_create_separate_rows() {
    let pool = setup_db().await;
    let app_id = unique_app();
    cleanup(&pool, &app_id).await;

    let ts1 = truncate_to_millis(millis_ts(1_700_000_000_000));
    let ts2 = truncate_to_millis(millis_ts(1_700_000_001_000));
    let p1 = json!({"id": "cust-003", "balance": 50});
    let p2 = json!({"id": "cust-003", "balance": 75});

    // Two observations for the same entity at different timestamps
    observations::upsert_observation(
        &pool,
        &app_id,
        "quickbooks",
        "customer",
        "cust-003",
        &compute_fingerprint(None, Some(ts1), &p1),
        ts1,
        &compute_comparable_hash(&p1, ts1),
        1,
        &p1,
        "test",
        false,
    )
    .await
    .expect("first");

    observations::upsert_observation(
        &pool,
        &app_id,
        "quickbooks",
        "customer",
        "cust-003",
        &compute_fingerprint(None, Some(ts2), &p2),
        ts2,
        &compute_comparable_hash(&p2, ts2),
        1,
        &p2,
        "test",
        false,
    )
    .await
    .expect("second");

    let count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM integrations_sync_observations WHERE app_id = $1")
            .bind(&app_id)
            .fetch_one(&pool)
            .await
            .expect("count");

    assert_eq!(count.0, 2, "distinct fingerprints must create two rows");

    cleanup(&pool, &app_id).await;
}

#[tokio::test]
#[serial]
async fn test_last_updated_time_stored_as_millisecond_precision() {
    let pool = setup_db().await;
    let app_id = unique_app();
    cleanup(&pool, &app_id).await;

    // Use a timestamp with sub-millisecond precision; truncation must happen
    // before insert so the DB CHECK constraint does not reject it.
    let precise_ts = Utc.timestamp_nanos(1_700_000_000_123_456_789_i64);
    let truncated_ts = truncate_to_millis(precise_ts);

    let payload = json!({"id": "cust-004"});
    let fp = compute_fingerprint(Some("st-ms-test"), None, &payload);
    let ch = compute_comparable_hash(&payload, truncated_ts);

    let row = observations::upsert_observation(
        &pool,
        &app_id,
        "quickbooks",
        "customer",
        "cust-004",
        &fp,
        truncated_ts,
        &ch,
        1,
        &payload,
        "test",
        false,
    )
    .await
    .expect("upsert");

    // Timestamp stored must equal the truncated value (milliseconds only).
    assert_eq!(
        row.last_updated_time.timestamp_millis(),
        truncated_ts.timestamp_millis()
    );
    assert_eq!(
        row.last_updated_time.timestamp_subsec_micros() % 1000,
        0,
        "stored last_updated_time must have zero sub-millisecond component"
    );

    cleanup(&pool, &app_id).await;
}

#[tokio::test]
#[serial]
async fn test_get_latest_for_entity_returns_highest_timestamp() {
    let pool = setup_db().await;
    let app_id = unique_app();
    cleanup(&pool, &app_id).await;

    let ts_old = truncate_to_millis(millis_ts(1_700_000_000_000));
    let ts_new = truncate_to_millis(millis_ts(1_700_000_005_000));

    let p_old = json!({"id": "inv-001", "status": "draft"});
    let p_new = json!({"id": "inv-001", "status": "sent"});

    observations::upsert_observation(
        &pool,
        &app_id,
        "quickbooks",
        "invoice",
        "inv-001",
        &compute_fingerprint(None, Some(ts_old), &p_old),
        ts_old,
        &compute_comparable_hash(&p_old, ts_old),
        1,
        &p_old,
        "test",
        false,
    )
    .await
    .expect("old");

    observations::upsert_observation(
        &pool,
        &app_id,
        "quickbooks",
        "invoice",
        "inv-001",
        &compute_fingerprint(None, Some(ts_new), &p_new),
        ts_new,
        &compute_comparable_hash(&p_new, ts_new),
        1,
        &p_new,
        "test",
        false,
    )
    .await
    .expect("new");

    let latest =
        observations::get_latest_for_entity(&pool, &app_id, "quickbooks", "invoice", "inv-001")
            .await
            .expect("get latest")
            .expect("must be Some");

    assert_eq!(
        latest.last_updated_time, ts_new,
        "must return the most recent observation"
    );

    cleanup(&pool, &app_id).await;
}

#[tokio::test]
#[serial]
async fn test_list_since_watermark() {
    let pool = setup_db().await;
    let app_id = unique_app();
    cleanup(&pool, &app_id).await;

    let ts_base = 1_700_000_000_000_i64;
    let payload = json!({"x": 1});

    for i in 0_i64..5 {
        let ts = truncate_to_millis(millis_ts(ts_base + i * 1_000));
        let fp = format!("ts:{}", ts.timestamp_millis());
        let ch = compute_comparable_hash(&payload, ts);
        observations::upsert_observation(
            &pool,
            &app_id,
            "quickbooks",
            "item",
            &format!("item-{i}"),
            &fp,
            ts,
            &ch,
            1,
            &payload,
            "test",
            false,
        )
        .await
        .expect("upsert");
    }

    // Watermark at i=2 → should return rows for i=2,3,4 (inclusive).
    let watermark = truncate_to_millis(millis_ts(ts_base + 2 * 1_000));
    let rows =
        observations::list_since_watermark(&pool, &app_id, "quickbooks", "item", watermark, 10)
            .await
            .expect("list");

    assert_eq!(rows.len(), 3, "rows since watermark should be 3 (i=2,3,4)");
    assert!(
        rows.iter().all(|r| r.last_updated_time >= watermark),
        "all rows must be at or after the watermark"
    );

    cleanup(&pool, &app_id).await;
}

#[tokio::test]
#[serial]
async fn test_find_by_comparable_hash() {
    let pool = setup_db().await;
    let app_id = unique_app();
    cleanup(&pool, &app_id).await;

    let ts = truncate_to_millis(millis_ts(1_700_000_010_000));
    let payload = json!({"amount": 500, "currency": "USD"});
    let ch = compute_comparable_hash(&payload, ts);
    let fp = format!("st:hash-lookup-test");

    observations::upsert_observation(
        &pool,
        &app_id,
        "quickbooks",
        "payment",
        "pay-001",
        &fp,
        ts,
        &ch,
        1,
        &payload,
        "test",
        false,
    )
    .await
    .expect("upsert");

    let found = observations::find_by_comparable_hash(&pool, &app_id, "quickbooks", "payment", &ch)
        .await
        .expect("find");

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].comparable_hash, ch);

    cleanup(&pool, &app_id).await;
}

#[tokio::test]
#[serial]
async fn test_payload_hash_fingerprint_prevents_key_collapse_on_distinct_payloads() {
    // Verify that two different payloads with no sync_token or timestamp get
    // distinct ph: fingerprints and are stored as separate rows.
    let pool = setup_db().await;
    let app_id = unique_app();
    cleanup(&pool, &app_id).await;

    let ts = truncate_to_millis(millis_ts(1_700_000_020_000));
    let p1 = json!({"balance": 100});
    let p2 = json!({"balance": 200});

    let fp1 = compute_fingerprint(None, None, &p1);
    let fp2 = compute_fingerprint(None, None, &p2);
    assert_ne!(
        fp1, fp2,
        "distinct payloads must produce distinct ph: fingerprints"
    );

    observations::upsert_observation(
        &pool,
        &app_id,
        "quickbooks",
        "vendor",
        "v-001",
        &fp1,
        ts,
        &compute_comparable_hash(&p1, ts),
        1,
        &p1,
        "test",
        false,
    )
    .await
    .expect("first");

    observations::upsert_observation(
        &pool,
        &app_id,
        "quickbooks",
        "vendor",
        "v-001",
        &fp2,
        ts,
        &compute_comparable_hash(&p2, ts),
        1,
        &p2,
        "test",
        false,
    )
    .await
    .expect("second");

    let count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM integrations_sync_observations WHERE app_id = $1")
            .bind(&app_id)
            .fetch_one(&pool)
            .await
            .expect("count");

    assert_eq!(
        count.0, 2,
        "two distinct payload hashes must produce two rows"
    );

    cleanup(&pool, &app_id).await;
}
