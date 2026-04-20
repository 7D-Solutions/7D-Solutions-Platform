use std::time::Duration;

use integrations_rs::domain::sync::push_attempts;
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
    format!("watchdog-test-{}", Uuid::new_v4().simple())
}

async fn cleanup(pool: &sqlx::PgPool, app_id: &str) {
    sqlx::query("DELETE FROM integrations_sync_push_attempts WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
}

async fn insert_inflight(
    pool: &sqlx::PgPool,
    app_id: &str,
    entity_id: &str,
    fp: &str,
) -> Uuid {
    let row = push_attempts::insert_attempt(
        pool, app_id, "quickbooks", "invoice", entity_id, "create", 1, fp,
    )
    .await
    .expect("insert");

    push_attempts::transition_to_inflight(pool, row.id)
        .await
        .expect("inflight")
        .expect("row");

    row.id
}

#[tokio::test]
#[serial]
async fn test_watchdog_times_out_stale_inflight() {
    let pool = setup_db().await;
    let app_id = unique_app();

    cleanup(&pool, &app_id).await;

    let stale_id = insert_inflight(&pool, &app_id, "inv-stale", "fp-stale-1").await;
    let fresh_id = insert_inflight(&pool, &app_id, "inv-fresh", "fp-fresh-1").await;

    // Backdate the stale row to 15 minutes ago
    sqlx::query(
        "UPDATE integrations_sync_push_attempts SET started_at = NOW() - INTERVAL '15 minutes' WHERE id = $1",
    )
    .bind(stale_id)
    .execute(&pool)
    .await
    .expect("backdate stale row");

    // Threshold: 10 minutes — only rows older than 10 min should be timed out
    let threshold = chrono::Utc::now() - chrono::Duration::minutes(10);
    let timed_out = push_attempts::timeout_stale_inflight(&pool, threshold)
        .await
        .expect("timeout_stale_inflight");

    assert_eq!(timed_out, 1, "exactly one stale row should have been timed out");

    // Stale row should now be 'failed' with error_message = 'inflight_timeout'
    let stale = push_attempts::get_attempt(&pool, stale_id)
        .await
        .expect("get stale")
        .expect("stale row exists");
    assert_eq!(stale.status, "failed");
    assert_eq!(stale.error_message.as_deref(), Some("inflight_timeout"));
    assert!(stale.completed_at.is_some());

    // Fresh row must remain 'inflight' — untouched by the watchdog
    let fresh = push_attempts::get_attempt(&pool, fresh_id)
        .await
        .expect("get fresh")
        .expect("fresh row exists");
    assert_eq!(fresh.status, "inflight", "fresh inflight row must not be timed out");
    assert!(fresh.error_message.is_none());

    cleanup(&pool, &app_id).await;
}

#[tokio::test]
#[serial]
async fn test_watchdog_noop_when_no_stale_rows() {
    let pool = setup_db().await;
    let app_id = unique_app();

    cleanup(&pool, &app_id).await;

    let threshold = chrono::Utc::now() - chrono::Duration::minutes(10);
    let timed_out = push_attempts::timeout_stale_inflight(&pool, threshold)
        .await
        .expect("timeout_stale_inflight");

    assert_eq!(timed_out, 0, "no stale rows — nothing should be timed out");

    cleanup(&pool, &app_id).await;
}

#[tokio::test]
#[serial]
async fn test_watchdog_timed_out_rows_allow_fresh_retry() {
    let pool = setup_db().await;
    let app_id = unique_app();

    cleanup(&pool, &app_id).await;

    let stale_id = insert_inflight(&pool, &app_id, "inv-timed-out", "fp-timeout-retry").await;

    // Backdate beyond 10 min threshold
    sqlx::query(
        "UPDATE integrations_sync_push_attempts SET started_at = NOW() - INTERVAL '20 minutes' WHERE id = $1",
    )
    .bind(stale_id)
    .execute(&pool)
    .await
    .expect("backdate");

    let threshold = chrono::Utc::now() - chrono::Duration::minutes(10);
    push_attempts::timeout_stale_inflight(&pool, threshold)
        .await
        .expect("timeout");

    // After timing out (status = 'failed'), a fresh attempt with the same fingerprint is allowed
    let retry = push_attempts::insert_attempt(
        &pool, &app_id, "quickbooks", "invoice", "inv-timed-out", "create", 1, "fp-timeout-retry",
    )
    .await
    .expect("fresh retry must succeed after timed-out row");

    assert_eq!(retry.status, "accepted");
    assert_ne!(retry.id, stale_id);

    cleanup(&pool, &app_id).await;
}
