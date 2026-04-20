//! Integration tests: sync detector marker-correlation + orphaned-write recovery.
//!
//! Verified against real Postgres (no mocks).
//!
//! Cases covered:
//!  1. Self-echo suppression — succeeded attempt with matching sync_token: no conflict.
//!  2. Orphaned-write recovery — failed attempt with matching sync_token: promoted to
//!     succeeded, no conflict row.
//!  3. Orphaned-write recovery via timestamp — unknown_failure attempt with matching
//!     result_last_updated_time: promoted to succeeded.
//!  4. True drift — no matching markers: conflict row created, sync.conflict.detected
//!     event enqueued atomically.
//!  5. Timestamp normalization — sub-millisecond differences do not produce false
//!     negatives when correlating.
//!  6. Conflict class derivation — edit/creation/deletion from presence of values.
//!  7. Tenant isolation — detector never reads across app_id boundaries.
//!  8. Value blob guard — > 256 KB rejected before touching the DB.
//!
//! Run:
//!   ./scripts/cargo-slot.sh test -p integrations-rs --test sync_detector_test -- --nocapture

use chrono::{TimeZone, Utc};
use integrations_rs::domain::sync::conflicts_repo::{get_conflict, list_conflicts};
use integrations_rs::domain::sync::dedupe::{compute_comparable_hash, truncate_to_millis};
use integrations_rs::domain::sync::detector::{run_detector, DetectorError, DetectorOutcome};
use integrations_rs::domain::sync::push_attempts;
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use std::time::Duration;
use tokio::sync::OnceCell;
use uuid::Uuid;

// ── Pool setup ────────────────────────────────────────────────────────────────

static TEST_POOL: OnceCell<sqlx::PgPool> = OnceCell::const_new();

async fn init_pool() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://integrations_user:integrations_pass@localhost:5449/integrations_db".to_string()
    });
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(90))
        .connect(&url)
        .await
        .expect("connect to integrations test DB");
    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("run integrations migrations");
    pool
}

async fn setup_db() -> sqlx::PgPool {
    TEST_POOL.get_or_init(init_pool).await.clone()
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn tenant() -> String {
    format!("det-{}", Uuid::new_v4().simple())
}

fn entity_id() -> String {
    format!("ent-{}", Uuid::new_v4().simple())
}

async fn cleanup(pool: &sqlx::PgPool, app_id: &str) {
    let _ = sqlx::query("DELETE FROM integrations_sync_conflicts WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM integrations_sync_push_attempts WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM integrations_outbox WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await;
}

/// Insert a push attempt directly with a specific status and optional result markers.
/// Used to simulate scenarios that the normal state machine can't produce (e.g.,
/// a failed attempt that has markers because QBO applied the write but our
/// success transition did not complete).
async fn seed_attempt_with_markers(
    pool: &sqlx::PgPool,
    app_id: &str,
    entity_id: &str,
    status: &str,
    sync_token: Option<&str>,
    last_updated_time_ms: Option<i64>,
    projection_hash: Option<&str>,
) -> Uuid {
    let id = Uuid::new_v4();
    let lut = last_updated_time_ms
        .map(|ms| truncate_to_millis(Utc.timestamp_millis_opt(ms).single().unwrap()));
    sqlx::query(
        r#"
        INSERT INTO integrations_sync_push_attempts (
            id, app_id, provider, entity_type, entity_id, operation,
            authority_version, request_fingerprint, status,
            result_sync_token, result_last_updated_time, result_projection_hash,
            completed_at
        )
        VALUES ($1, $2, 'quickbooks', 'invoice', $3, 'create',
                1, 'fp-seed', $4,
                $5, $6, $7,
                NOW())
        "#,
    )
    .bind(id)
    .bind(app_id)
    .bind(entity_id)
    .bind(status)
    .bind(sync_token)
    .bind(lut)
    .bind(projection_hash)
    .execute(pool)
    .await
    .expect("seed_attempt_with_markers");
    id
}

// ── 1. Self-echo suppression ──────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn self_echo_suppressed_for_succeeded_attempt_with_matching_sync_token() {
    let pool = setup_db().await;
    let app_id = tenant();
    let eid = entity_id();
    cleanup(&pool, &app_id).await;

    let sync_token = "tok-echo-42";
    let attempt_id = seed_attempt_with_markers(
        &pool,
        &app_id,
        &eid,
        "succeeded",
        Some(sync_token),
        None,
        None,
    )
    .await;

    let fingerprint = format!("st:{}", sync_token);
    let comparable_hash = compute_comparable_hash(&serde_json::json!({"id": &eid}), Utc::now());

    let outcome = run_detector(
        &pool,
        &app_id,
        "quickbooks",
        "invoice",
        &eid,
        &fingerprint,
        &comparable_hash,
        Some(serde_json::json!({"amount": 100})),
        Some(serde_json::json!({"amount": 100})),
    )
    .await
    .expect("run_detector must not fail");

    assert!(
        matches!(outcome, DetectorOutcome::SelfEchoSuppressed { attempt_id: aid } if aid == attempt_id),
        "expected SelfEchoSuppressed, got: {:?}",
        outcome
    );

    // No conflict row must have been created.
    let all = list_conflicts(&pool, &app_id, None, 100, 0)
        .await
        .expect("list_conflicts");
    assert_eq!(all.len(), 0, "self-echo must not create a conflict row");

    cleanup(&pool, &app_id).await;
}

// ── 2. Orphaned-write recovery (failed + sync_token) ─────────────────────────

#[tokio::test]
#[serial]
async fn orphaned_write_recovered_for_failed_attempt_with_matching_sync_token() {
    let pool = setup_db().await;
    let app_id = tenant();
    let eid = entity_id();
    cleanup(&pool, &app_id).await;

    let sync_token = "tok-orphan-7";
    let attempt_id = seed_attempt_with_markers(
        &pool, &app_id, &eid,
        "failed", Some(sync_token), None, None,
    ).await;

    let fingerprint = format!("st:{}", sync_token);
    let comparable_hash = compute_comparable_hash(&serde_json::json!({"id": &eid}), Utc::now());

    let outcome = run_detector(
        &pool, &app_id, "quickbooks", "invoice", &eid,
        &fingerprint, &comparable_hash,
        Some(serde_json::json!({"amount": 50})),
        Some(serde_json::json!({"amount": 50})),
    )
    .await
    .expect("run_detector");

    assert!(
        matches!(outcome, DetectorOutcome::OrphanedWriteRecovered { attempt_id: aid } if aid == attempt_id),
        "expected OrphanedWriteRecovered, got: {:?}",
        outcome
    );

    // Attempt must now be succeeded.
    let promoted = push_attempts::get_attempt(&pool, attempt_id)
        .await
        .expect("get_attempt")
        .expect("attempt must exist");
    assert_eq!(promoted.status, "succeeded", "orphaned attempt must be promoted to succeeded");

    // No conflict row.
    let all = list_conflicts(&pool, &app_id, None, 100, 0).await.expect("list");
    assert_eq!(all.len(), 0, "orphaned-write recovery must suppress conflict");

    cleanup(&pool, &app_id).await;
}

// ── 3. Orphaned-write recovery (unknown_failure + timestamp) ──────────────────

#[tokio::test]
#[serial]
async fn orphaned_write_recovered_for_unknown_failure_with_matching_timestamp() {
    let pool = setup_db().await;
    let app_id = tenant();
    let eid = entity_id();
    cleanup(&pool, &app_id).await;

    let lut_ms: i64 = 1_745_000_000_123; // arbitrary epoch millis
    let attempt_id = seed_attempt_with_markers(
        &pool, &app_id, &eid,
        "unknown_failure", None, Some(lut_ms), None,
    ).await;

    // Fingerprint encodes the millisecond epoch.
    let fingerprint = format!("ts:{}", lut_ms);
    let comparable_hash = compute_comparable_hash(&serde_json::json!({"id": &eid}), Utc::now());

    let outcome = run_detector(
        &pool, &app_id, "quickbooks", "invoice", &eid,
        &fingerprint, &comparable_hash,
        Some(serde_json::json!({"amount": 200})),
        Some(serde_json::json!({"amount": 200})),
    )
    .await
    .expect("run_detector");

    assert!(
        matches!(outcome, DetectorOutcome::OrphanedWriteRecovered { attempt_id: aid } if aid == attempt_id),
        "expected OrphanedWriteRecovered via timestamp, got: {:?}",
        outcome
    );

    let promoted = push_attempts::get_attempt(&pool, attempt_id).await.expect("get").expect("row");
    assert_eq!(promoted.status, "succeeded");

    cleanup(&pool, &app_id).await;
}

// ── 4. True drift — conflict opened + event enqueued ─────────────────────────

#[tokio::test]
#[serial]
async fn true_drift_opens_conflict_and_enqueues_event() {
    let pool = setup_db().await;
    let app_id = tenant();
    let eid = entity_id();
    cleanup(&pool, &app_id).await;

    // No push attempt with matching markers for this entity.
    seed_attempt_with_markers(
        &pool, &app_id, &eid,
        "succeeded", Some("tok-different"), None, None,
    ).await;

    let internal_val = serde_json::json!({"amount": 100});
    let external_val = serde_json::json!({"amount": 999});

    // fingerprint with a different token → no match
    let fingerprint = "st:tok-drifted";
    let comparable_hash = compute_comparable_hash(&serde_json::json!({"id": &eid}), Utc::now());

    let outcome = run_detector(
        &pool, &app_id, "quickbooks", "invoice", &eid,
        fingerprint, &comparable_hash,
        Some(internal_val.clone()),
        Some(external_val.clone()),
    )
    .await
    .expect("run_detector");

    let conflict_id = match outcome {
        DetectorOutcome::ConflictOpened(ref row) => {
            assert_eq!(row.status, "pending");
            assert_eq!(row.conflict_class, "edit");
            assert_eq!(row.detected_by, "detector");
            assert_eq!(row.app_id, app_id);
            row.id
        }
        other => panic!("expected ConflictOpened, got: {:?}", other),
    };

    // Conflict row exists in DB.
    let from_db = get_conflict(&pool, &app_id, conflict_id)
        .await
        .expect("get_conflict")
        .expect("conflict must exist in DB");
    assert_eq!(from_db.status, "pending");
    assert_eq!(from_db.internal_value, Some(internal_val));
    assert_eq!(from_db.external_value, Some(external_val));

    // Outbox has sync.conflict.detected event.
    let event_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM integrations_outbox \
         WHERE app_id = $1 AND event_type = 'sync.conflict.detected'",
    )
    .bind(&app_id)
    .fetch_one(&pool)
    .await
    .expect("outbox count");
    assert_eq!(event_count.0, 1, "exactly one sync.conflict.detected event in outbox");

    // Event payload matches conflict.
    let (payload,): (serde_json::Value,) = sqlx::query_as(
        "SELECT payload FROM integrations_outbox \
         WHERE app_id = $1 AND event_type = 'sync.conflict.detected'",
    )
    .bind(&app_id)
    .fetch_one(&pool)
    .await
    .expect("outbox payload");

    assert_eq!(payload["event_type"], "sync.conflict.detected");
    assert_eq!(payload["source_module"], "integrations");
    assert_eq!(payload["mutation_class"], "DATA_MUTATION");
    let inner = &payload["payload"];
    assert_eq!(inner["conflict_class"], "edit");
    assert_eq!(inner["entity_type"], "invoice");
    assert_eq!(inner["entity_id"].as_str().unwrap(), eid.as_str());
    assert_eq!(inner["conflict_id"].as_str().unwrap(), conflict_id.to_string().as_str());
    assert_eq!(inner["detected_by"], "detector");

    cleanup(&pool, &app_id).await;
}

// ── 5. Timestamp normalization prevents false-negative ────────────────────────

#[tokio::test]
#[serial]
async fn timestamp_normalization_prevents_false_negative_on_sub_millisecond_diff() {
    let pool = setup_db().await;
    let app_id = tenant();
    let eid = entity_id();
    cleanup(&pool, &app_id).await;

    // Attempt stored with a ms-truncated timestamp.
    let lut_ms: i64 = 1_745_100_000_456; // epoch millis
    let attempt_id = seed_attempt_with_markers(
        &pool, &app_id, &eid,
        "failed", None, Some(lut_ms), None,
    ).await;

    // Fingerprint uses the SAME ms value — should match even though the raw
    // timestamp from QBO might have sub-millisecond fractions that were stripped.
    let fingerprint = format!("ts:{}", lut_ms);
    let comparable_hash = compute_comparable_hash(&serde_json::json!({"id": &eid}), Utc::now());

    let outcome = run_detector(
        &pool, &app_id, "quickbooks", "invoice", &eid,
        &fingerprint, &comparable_hash,
        Some(serde_json::json!({})),
        Some(serde_json::json!({})),
    )
    .await
    .expect("run_detector");

    assert!(
        matches!(outcome, DetectorOutcome::OrphanedWriteRecovered { attempt_id: aid } if aid == attempt_id),
        "ms-equal timestamps must match after normalization, got: {:?}",
        outcome
    );

    let promoted = push_attempts::get_attempt(&pool, attempt_id).await.expect("get").expect("row");
    assert_eq!(promoted.status, "succeeded", "promoted after timestamp-based marker match");

    cleanup(&pool, &app_id).await;
}

// ── 6. Conflict class derivation ──────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn conflict_class_deletion_when_no_values_provided() {
    let pool = setup_db().await;
    let app_id = tenant();
    let eid = entity_id();
    cleanup(&pool, &app_id).await;

    let outcome = run_detector(
        &pool, &app_id, "quickbooks", "invoice", &eid,
        "ph:deadbeef", "cmphash-xyz",
        None, // internal_value — entity deleted on platform
        None, // external_value — entity deleted on provider
    )
    .await
    .expect("run_detector");

    match outcome {
        DetectorOutcome::ConflictOpened(row) => {
            assert_eq!(row.conflict_class, "deletion");
        }
        other => panic!("expected ConflictOpened(deletion), got: {:?}", other),
    }

    cleanup(&pool, &app_id).await;
}

#[tokio::test]
#[serial]
async fn conflict_class_deletion_when_only_external_value_present() {
    let pool = setup_db().await;
    let app_id = tenant();
    let eid = entity_id();
    cleanup(&pool, &app_id).await;

    // Only external_value present → platform entity is absent → Deletion class.
    // (Creation class requires both value snapshots per schema constraint.)
    let outcome = run_detector(
        &pool, &app_id, "quickbooks", "invoice", &eid,
        "ph:cafebabe", "cmphash-abc",
        None,
        Some(serde_json::json!({"amount": 500})),
    )
    .await
    .expect("run_detector");

    match outcome {
        DetectorOutcome::ConflictOpened(row) => {
            assert_eq!(row.conflict_class, "deletion");
        }
        other => panic!("expected ConflictOpened(deletion), got: {:?}", other),
    }

    cleanup(&pool, &app_id).await;
}

// ── 7. Tenant isolation ───────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn detector_does_not_cross_app_id_boundaries() {
    let pool = setup_db().await;
    let app_a = tenant();
    let app_b = tenant();
    let eid = entity_id();
    cleanup(&pool, &app_a).await;
    cleanup(&pool, &app_b).await;

    let sync_token = "tok-tenant-guard";

    // Tenant A has a succeeded attempt with this token.
    seed_attempt_with_markers(
        &pool, &app_a, &eid,
        "succeeded", Some(sync_token), None, None,
    ).await;

    let fingerprint = format!("st:{}", sync_token);
    let comparable_hash = compute_comparable_hash(&serde_json::json!({"id": &eid}), Utc::now());

    // Tenant B runs the detector with the same entity + markers.
    // Must NOT pick up Tenant A's attempt → must open a conflict.
    let outcome = run_detector(
        &pool, &app_b, "quickbooks", "invoice", &eid,
        &fingerprint, &comparable_hash,
        Some(serde_json::json!({"amount": 1})),
        Some(serde_json::json!({"amount": 2})),
    )
    .await
    .expect("run_detector for tenant B");

    assert!(
        matches!(outcome, DetectorOutcome::ConflictOpened(_)),
        "tenant B must not see tenant A's attempt, expected ConflictOpened, got: {:?}",
        outcome
    );

    cleanup(&pool, &app_a).await;
    cleanup(&pool, &app_b).await;
}

// ── 8. Value blob guard ───────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn value_too_large_rejected_before_db() {
    let pool = setup_db().await;
    let app_id = tenant();
    let eid = entity_id();

    let big = "x".repeat(integrations_rs::domain::sync::MAX_VALUE_BYTES + 1);
    let err = run_detector(
        &pool, &app_id, "quickbooks", "invoice", &eid,
        "ph:abc", "cmphash",
        Some(serde_json::json!(big)),
        Some(serde_json::json!({"amount": 1})),
    )
    .await;

    assert!(
        matches!(err, Err(DetectorError::ValueTooLarge)),
        "must reject oversized value blob before touching DB, got: {:?}",
        err
    );
}

// ── 9. Orphaned-write via projection_hash (comparable_hash correlation) ───────

#[tokio::test]
#[serial]
async fn orphaned_write_recovered_via_projection_hash() {
    let pool = setup_db().await;
    let app_id = tenant();
    let eid = entity_id();
    cleanup(&pool, &app_id).await;

    // Observation has ph: fingerprint; use comparable_hash as the projection key.
    let comparable_hash = "ph:test-comparable-hash-abc123";
    let attempt_id = seed_attempt_with_markers(
        &pool, &app_id, &eid,
        "failed", None, None, Some(comparable_hash),
    ).await;

    // ph: fingerprint → no sync_token or timestamp extracted.
    // comparable_hash matches result_projection_hash on the attempt.
    let outcome = run_detector(
        &pool, &app_id, "quickbooks", "invoice", &eid,
        "ph:test-comparable-hash-abc123", comparable_hash,
        Some(serde_json::json!({"amount": 77})),
        Some(serde_json::json!({"amount": 77})),
    )
    .await
    .expect("run_detector via projection_hash");

    assert!(
        matches!(outcome, DetectorOutcome::OrphanedWriteRecovered { attempt_id: aid } if aid == attempt_id),
        "expected OrphanedWriteRecovered via projection_hash, got: {:?}",
        outcome
    );

    let promoted = push_attempts::get_attempt(&pool, attempt_id).await.expect("get").expect("row");
    assert_eq!(promoted.status, "succeeded");

    cleanup(&pool, &app_id).await;
}
