//! Integration tests for sync_pull_recovery::reconcile_orphan_inflight_pulls (bd-vbedx).
//!
//! Verifies:
//!  1.  orphan_reconcile_marks_old_inflight_failed — rows older than threshold are marked failed.
//!  2.  orphan_reconcile_preserves_young_inflight  — rows younger than threshold are untouched.
//!  3.  orphan_reconcile_respects_threshold_argument — custom threshold controls the cutoff.
//!
//! Run:
//!   ./scripts/cargo-slot.sh test -p integrations-rs --test orphan_reconcile_test

use integrations_rs::sync_pull_recovery::reconcile_orphan_inflight_pulls;
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://integrations_user:integrations_pass@localhost:5449/integrations_db"
            .to_string()
    });
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&url)
        .await
        .expect("Failed to connect to integrations test DB");
    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run integrations migrations");
    pool
}

fn unique_app_id(tag: &str) -> String {
    format!("test-{}-{}", tag, Uuid::new_v4().simple())
}

// ── 1. Old inflight row is marked failed ─────────────────────────────────────

#[tokio::test]
#[serial]
async fn orphan_reconcile_marks_old_inflight_failed() {
    let pool = setup_db().await;
    let app_id = unique_app_id("old-inflight");

    sqlx::query(
        "INSERT INTO integrations_sync_pull_log (app_id, entity_type, triggered_by, started_at, status)
         VALUES ($1, 'invoice', 'test-user', now() - interval '15 minutes', 'inflight')",
    )
    .bind(&app_id)
    .execute(&pool)
    .await
    .expect("insert orphan row");

    let count = reconcile_orphan_inflight_pulls(&pool, 600)
        .await
        .expect("reconcile must not error");

    assert_eq!(count, 1, "must return 1 reconciled row");

    let row: (String, Option<String>, Option<chrono::DateTime<chrono::Utc>>) = sqlx::query_as(
        "SELECT status, error, completed_at FROM integrations_sync_pull_log WHERE app_id = $1",
    )
    .bind(&app_id)
    .fetch_one(&pool)
    .await
    .expect("fetch row");

    assert_eq!(row.0, "failed", "status must be 'failed'");
    assert_eq!(row.1.as_deref(), Some("service_restart"), "error must be 'service_restart'");
    assert!(row.2.is_some(), "completed_at must be set");
}

// ── 2. Young inflight row is preserved ───────────────────────────────────────

#[tokio::test]
#[serial]
async fn orphan_reconcile_preserves_young_inflight() {
    let pool = setup_db().await;
    let app_id = unique_app_id("young-inflight");

    sqlx::query(
        "INSERT INTO integrations_sync_pull_log (app_id, entity_type, triggered_by, started_at, status)
         VALUES ($1, 'invoice', 'test-user', now() - interval '5 minutes', 'inflight')",
    )
    .bind(&app_id)
    .execute(&pool)
    .await
    .expect("insert young row");

    let count = reconcile_orphan_inflight_pulls(&pool, 600)
        .await
        .expect("reconcile must not error");

    assert_eq!(count, 0, "must return 0 — row is younger than threshold");

    let status: String =
        sqlx::query_scalar("SELECT status FROM integrations_sync_pull_log WHERE app_id = $1")
            .bind(&app_id)
            .fetch_one(&pool)
            .await
            .expect("fetch row");

    assert_eq!(status, "inflight", "young row must remain 'inflight'");
}

// ── 3. Custom threshold controls cutoff ───────────────────────────────────────

#[tokio::test]
#[serial]
async fn orphan_reconcile_respects_threshold_argument() {
    let pool = setup_db().await;
    let app_id = unique_app_id("custom-threshold");

    sqlx::query(
        "INSERT INTO integrations_sync_pull_log (app_id, entity_type, triggered_by, started_at, status)
         VALUES ($1, 'invoice', 'test-user', now() - interval '3 minutes', 'inflight')",
    )
    .bind(&app_id)
    .execute(&pool)
    .await
    .expect("insert row");

    // threshold = 120s (2 min); row is 3 min old → must be reconciled
    let count = reconcile_orphan_inflight_pulls(&pool, 120)
        .await
        .expect("reconcile must not error");

    assert_eq!(count, 1, "must return 1 — row exceeds 2-minute threshold");

    let status: String =
        sqlx::query_scalar("SELECT status FROM integrations_sync_pull_log WHERE app_id = $1")
            .bind(&app_id)
            .fetch_one(&pool)
            .await
            .expect("fetch row");

    assert_eq!(status, "failed");
}
