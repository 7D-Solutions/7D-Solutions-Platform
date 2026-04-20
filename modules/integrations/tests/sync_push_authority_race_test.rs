//! Integration tests for push-attempt authority race handling.
//!
//! Tests run against a real Postgres instance. No mocks, no stubs.
//! Covers two state-machine branches:
//!   pre-call  — pre_call_version_check short-circuits to `superseded`
//!   post-call — post_call_reconcile records `completed_under_stale_authority`
//!               then auto-closes (equal values) or opens a conflict (divergent values)

use std::time::Duration;

use integrations_rs::domain::sync::push_attempts::{
    self, post_call_reconcile, pre_call_version_check, PreCallOutcome, PushStatus, ReconcileOutcome,
};
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
        .max_connections(4)
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
    format!("race-test-{}", Uuid::new_v4().simple())
}

async fn cleanup(pool: &sqlx::PgPool, app_id: &str) {
    sqlx::query("DELETE FROM integrations_sync_conflicts WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM integrations_sync_push_attempts WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
}

/// Insert an attempt stamped with `authority_version` and return its id.
async fn seed_attempt(
    pool: &sqlx::PgPool,
    app_id: &str,
    entity_id: &str,
    fp: &str,
    authority_version: i64,
) -> Uuid {
    push_attempts::insert_attempt(
        pool,
        app_id,
        "quickbooks",
        "invoice",
        entity_id,
        "update",
        authority_version,
        fp,
    )
    .await
    .expect("insert attempt")
    .id
}

/// Transition an attempt to inflight and return its id.
async fn make_inflight(pool: &sqlx::PgPool, attempt_id: Uuid) {
    push_attempts::transition_to_inflight(pool, attempt_id)
        .await
        .expect("transition_to_inflight")
        .expect("attempt row must exist");
}

// ============================================================================
// Pre-call: version check
// ============================================================================

#[tokio::test]
#[serial]
async fn pre_call_supersedes_when_authority_version_advanced() {
    let pool = setup_db().await;
    let app_id = unique_app();
    cleanup(&pool, &app_id).await;

    // Attempt stamped at version 1; authority has since advanced to version 2.
    let attempt_id = seed_attempt(&pool, &app_id, "inv-race-1", "fp-race-1", 1).await;

    let outcome = pre_call_version_check(&pool, attempt_id, 2)
        .await
        .expect("pre_call_version_check");

    match outcome {
        PreCallOutcome::Superseded(row) => {
            assert_eq!(row.id, attempt_id);
            assert_eq!(row.status, "superseded");
            assert!(row.completed_at.is_some(), "superseded attempt must have completed_at set");
        }
        PreCallOutcome::ReadyForInflight => panic!("expected Superseded, got ReadyForInflight"),
    }

    // Verify the DB row is now 'superseded'.
    let fetched = push_attempts::get_attempt(&pool, attempt_id)
        .await
        .expect("get_attempt")
        .expect("row must exist");
    assert_eq!(fetched.status, "superseded");

    cleanup(&pool, &app_id).await;
}

#[tokio::test]
#[serial]
async fn pre_call_ready_when_authority_version_matches() {
    let pool = setup_db().await;
    let app_id = unique_app();
    cleanup(&pool, &app_id).await;

    let attempt_id = seed_attempt(&pool, &app_id, "inv-race-2", "fp-race-2", 3).await;

    let outcome = pre_call_version_check(&pool, attempt_id, 3)
        .await
        .expect("pre_call_version_check");

    assert!(
        matches!(outcome, PreCallOutcome::ReadyForInflight),
        "matching version must return ReadyForInflight"
    );

    // Attempt must still be 'accepted' — not mutated.
    let fetched = push_attempts::get_attempt(&pool, attempt_id)
        .await
        .expect("get_attempt")
        .expect("row must exist");
    assert_eq!(fetched.status, "accepted");

    cleanup(&pool, &app_id).await;
}

#[tokio::test]
#[serial]
async fn superseded_attempt_allows_fresh_retry() {
    let pool = setup_db().await;
    let app_id = unique_app();
    cleanup(&pool, &app_id).await;

    let attempt_id = seed_attempt(&pool, &app_id, "inv-race-3", "fp-race-3", 1).await;

    // Supersede the attempt.
    let outcome = pre_call_version_check(&pool, attempt_id, 2)
        .await
        .expect("pre_call_version_check");
    assert!(matches!(outcome, PreCallOutcome::Superseded(_)));

    // A new attempt for the same entity+fingerprint is permitted because
    // 'superseded' is excluded from the dedup unique index.
    let retry = push_attempts::insert_attempt(
        &pool,
        &app_id,
        "quickbooks",
        "invoice",
        "inv-race-3",
        "update",
        2, // current authority version
        "fp-race-3",
    )
    .await
    .expect("fresh retry after supersede must succeed");

    assert_eq!(retry.status, "accepted");
    assert_ne!(retry.id, attempt_id);

    cleanup(&pool, &app_id).await;
}

// ============================================================================
// Post-call: stale-authority reconciliation
// ============================================================================

#[tokio::test]
#[serial]
async fn post_call_reconcile_auto_closes_when_values_equal() {
    let pool = setup_db().await;
    let app_id = unique_app();
    cleanup(&pool, &app_id).await;

    let attempt_id = seed_attempt(&pool, &app_id, "inv-recon-1", "fp-recon-1", 1).await;
    make_inflight(&pool, attempt_id).await;

    let val = serde_json::json!({"amount": 100, "customer": "ACME"});

    let outcome = post_call_reconcile(
        &pool,
        attempt_id,
        &app_id,
        "quickbooks",
        "invoice",
        "inv-recon-1",
        Some(val.clone()),
        Some(val.clone()),
    )
    .await
    .expect("post_call_reconcile");

    assert!(
        matches!(outcome, ReconcileOutcome::AutoClosed),
        "equal values must auto-close"
    );

    // Attempt must be marked completed_under_stale_authority.
    let fetched = push_attempts::get_attempt(&pool, attempt_id)
        .await
        .expect("get_attempt")
        .expect("row");
    assert_eq!(fetched.status, "completed_under_stale_authority");
    assert!(fetched.completed_at.is_some());

    // No conflict row created.
    let conflict_count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM integrations_sync_conflicts WHERE app_id = $1")
            .bind(&app_id)
            .fetch_one(&pool)
            .await
            .expect("count conflicts");
    assert_eq!(conflict_count.0, 0, "no conflict should be created for auto-close");

    cleanup(&pool, &app_id).await;
}

#[tokio::test]
#[serial]
async fn post_call_reconcile_opens_conflict_when_values_diverge() {
    let pool = setup_db().await;
    let app_id = unique_app();
    cleanup(&pool, &app_id).await;

    let attempt_id = seed_attempt(&pool, &app_id, "inv-recon-2", "fp-recon-2", 1).await;
    make_inflight(&pool, attempt_id).await;

    let internal = serde_json::json!({"amount": 100, "customer": "ACME"});
    let external = serde_json::json!({"amount": 150, "customer": "ACME"});

    let outcome = post_call_reconcile(
        &pool,
        attempt_id,
        &app_id,
        "quickbooks",
        "invoice",
        "inv-recon-2",
        Some(internal),
        Some(external),
    )
    .await
    .expect("post_call_reconcile");

    match outcome {
        ReconcileOutcome::ConflictOpened(conflict) => {
            assert_eq!(conflict.app_id, app_id);
            assert_eq!(conflict.entity_id, "inv-recon-2");
            assert_eq!(conflict.conflict_class, "edit");
            assert_eq!(conflict.status, "pending");
            assert!(
                conflict.detected_by.contains("push_attempt:"),
                "detected_by must reference the push attempt"
            );
        }
        ReconcileOutcome::AutoClosed => panic!("divergent values must open a conflict"),
    }

    // Attempt must be completed_under_stale_authority.
    let fetched = push_attempts::get_attempt(&pool, attempt_id)
        .await
        .expect("get_attempt")
        .expect("row");
    assert_eq!(fetched.status, "completed_under_stale_authority");

    // Exactly one conflict row created.
    let conflict_count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM integrations_sync_conflicts WHERE app_id = $1")
            .bind(&app_id)
            .fetch_one(&pool)
            .await
            .expect("count conflicts");
    assert_eq!(conflict_count.0, 1);

    cleanup(&pool, &app_id).await;
}

#[tokio::test]
#[serial]
async fn post_call_reconcile_auto_closes_when_both_values_are_none() {
    let pool = setup_db().await;
    let app_id = unique_app();
    cleanup(&pool, &app_id).await;

    let attempt_id = seed_attempt(&pool, &app_id, "inv-recon-3", "fp-recon-3", 1).await;
    make_inflight(&pool, attempt_id).await;

    let outcome = post_call_reconcile(
        &pool,
        attempt_id,
        &app_id,
        "quickbooks",
        "invoice",
        "inv-recon-3",
        None,
        None,
    )
    .await
    .expect("post_call_reconcile");

    assert!(
        matches!(outcome, ReconcileOutcome::AutoClosed),
        "both-None values are equal — must auto-close"
    );

    cleanup(&pool, &app_id).await;
}

#[tokio::test]
#[serial]
async fn post_call_reconcile_opens_conflict_when_one_side_is_none() {
    let pool = setup_db().await;
    let app_id = unique_app();
    cleanup(&pool, &app_id).await;

    let attempt_id = seed_attempt(&pool, &app_id, "inv-recon-4", "fp-recon-4", 1).await;
    make_inflight(&pool, attempt_id).await;

    // internal has data, external is absent — clearly divergent.
    let outcome = post_call_reconcile(
        &pool,
        attempt_id,
        &app_id,
        "quickbooks",
        "invoice",
        "inv-recon-4",
        Some(serde_json::json!({"amount": 200})),
        None,
    )
    .await
    .expect("post_call_reconcile");

    match outcome {
        ReconcileOutcome::ConflictOpened(conflict) => {
            assert_eq!(conflict.conflict_class, "deletion");
        }
        ReconcileOutcome::AutoClosed => panic!("one-sided None must open a conflict"),
    }

    cleanup(&pool, &app_id).await;
}

// ============================================================================
// PushStatus round-trip
// ============================================================================

#[test]
fn push_status_new_variants_round_trip() {
    for (status, expected) in [
        (PushStatus::Superseded, "superseded"),
        (PushStatus::CompletedUnderStaleAuthority, "completed_under_stale_authority"),
    ] {
        let s = status.as_str();
        assert_eq!(s, expected);
        let back = PushStatus::from_str(s).expect("from_str must round-trip");
        assert_eq!(back.as_str(), expected);
    }
}
