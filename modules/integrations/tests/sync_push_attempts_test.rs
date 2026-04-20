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
    format!("push-test-{}", Uuid::new_v4().simple())
}

async fn cleanup(pool: &sqlx::PgPool, app_id: &str) {
    sqlx::query("DELETE FROM integrations_sync_push_attempts WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
}

#[tokio::test]
#[serial]
async fn test_insert_attempt_starts_as_accepted() {
    let pool = setup_db().await;
    let app_id = unique_app();

    cleanup(&pool, &app_id).await;

    let row = push_attempts::insert_attempt(
        &pool,
        &app_id,
        "quickbooks",
        "invoice",
        "inv-001",
        "create",
        1,
        "fp-abc123",
    )
    .await
    .expect("insert attempt");

    assert_eq!(row.status, "accepted");
    assert!(row.completed_at.is_none());
    assert!(row.error_message.is_none());
    assert_eq!(row.authority_version, 1);
    assert_eq!(row.request_fingerprint, "fp-abc123");

    cleanup(&pool, &app_id).await;
}

#[tokio::test]
#[serial]
async fn test_transition_accepted_to_inflight() {
    let pool = setup_db().await;
    let app_id = unique_app();

    cleanup(&pool, &app_id).await;

    let row = push_attempts::insert_attempt(
        &pool, &app_id, "quickbooks", "invoice", "inv-002", "update", 1, "fp-def456",
    )
    .await
    .expect("insert");

    let inflight = push_attempts::transition_to_inflight(&pool, row.id)
        .await
        .expect("transition")
        .expect("row must be returned");

    assert_eq!(inflight.status, "inflight");
    assert_eq!(inflight.id, row.id);

    cleanup(&pool, &app_id).await;
}

#[tokio::test]
#[serial]
async fn test_complete_attempt_to_succeeded() {
    let pool = setup_db().await;
    let app_id = unique_app();

    cleanup(&pool, &app_id).await;

    let row = push_attempts::insert_attempt(
        &pool, &app_id, "quickbooks", "bill", "bill-001", "create", 2, "fp-ghi789",
    )
    .await
    .expect("insert");

    push_attempts::transition_to_inflight(&pool, row.id)
        .await
        .expect("transition to inflight");

    let done = push_attempts::complete_attempt(&pool, row.id, "succeeded", None)
        .await
        .expect("complete")
        .expect("row");

    assert_eq!(done.status, "succeeded");
    assert!(done.completed_at.is_some());
    assert!(done.error_message.is_none());

    cleanup(&pool, &app_id).await;
}

#[tokio::test]
#[serial]
async fn test_complete_attempt_to_failed_with_error_message() {
    let pool = setup_db().await;
    let app_id = unique_app();

    cleanup(&pool, &app_id).await;

    let row = push_attempts::insert_attempt(
        &pool, &app_id, "quickbooks", "payment", "pay-001", "delete", 3, "fp-jkl012",
    )
    .await
    .expect("insert");

    push_attempts::transition_to_inflight(&pool, row.id)
        .await
        .expect("inflight");

    let done = push_attempts::complete_attempt(&pool, row.id, "failed", Some("provider rejected: 400"))
        .await
        .expect("complete")
        .expect("row");

    assert_eq!(done.status, "failed");
    assert_eq!(done.error_message.as_deref(), Some("provider rejected: 400"));

    cleanup(&pool, &app_id).await;
}

#[tokio::test]
#[serial]
async fn test_complete_attempt_to_unknown_failure() {
    let pool = setup_db().await;
    let app_id = unique_app();

    cleanup(&pool, &app_id).await;

    let row = push_attempts::insert_attempt(
        &pool, &app_id, "quickbooks", "vendor", "vendor-001", "update", 1, "fp-mno345",
    )
    .await
    .expect("insert");

    push_attempts::transition_to_inflight(&pool, row.id)
        .await
        .expect("inflight");

    let done = push_attempts::complete_attempt(&pool, row.id, "unknown_failure", Some("timeout"))
        .await
        .expect("complete")
        .expect("row");

    assert_eq!(done.status, "unknown_failure");
    assert_eq!(done.error_message.as_deref(), Some("timeout"));

    cleanup(&pool, &app_id).await;
}

#[tokio::test]
#[serial]
async fn test_partial_unique_blocks_duplicate_accepted_intent() {
    let pool = setup_db().await;
    let app_id = unique_app();

    cleanup(&pool, &app_id).await;

    push_attempts::insert_attempt(
        &pool, &app_id, "quickbooks", "invoice", "inv-dup", "create", 1, "fp-same",
    )
    .await
    .expect("first insert");

    // Second identical intent while first is 'accepted' must fail
    let err = push_attempts::insert_attempt(
        &pool, &app_id, "quickbooks", "invoice", "inv-dup", "create", 1, "fp-same",
    )
    .await
    .expect_err("duplicate must fail");

    let db_err = err.to_string();
    assert!(
        db_err.contains("integrations_sync_push_attempts_intent_unique"),
        "expected unique constraint violation, got: {db_err}"
    );

    cleanup(&pool, &app_id).await;
}

#[tokio::test]
#[serial]
async fn test_partial_unique_allows_retry_after_failed() {
    let pool = setup_db().await;
    let app_id = unique_app();

    cleanup(&pool, &app_id).await;

    let row = push_attempts::insert_attempt(
        &pool, &app_id, "quickbooks", "invoice", "inv-retry", "create", 1, "fp-retry",
    )
    .await
    .expect("first insert");

    push_attempts::transition_to_inflight(&pool, row.id)
        .await
        .expect("inflight");

    push_attempts::complete_attempt(&pool, row.id, "failed", Some("404 not found"))
        .await
        .expect("failed");

    // After 'failed', a fresh attempt with same fingerprint is allowed
    let retry = push_attempts::insert_attempt(
        &pool, &app_id, "quickbooks", "invoice", "inv-retry", "create", 1, "fp-retry",
    )
    .await
    .expect("retry insert must succeed");

    assert_eq!(retry.status, "accepted");
    assert_ne!(retry.id, row.id);

    cleanup(&pool, &app_id).await;
}

#[tokio::test]
#[serial]
async fn test_list_stale_inflight_returns_old_attempts() {
    let pool = setup_db().await;
    let app_id = unique_app();

    cleanup(&pool, &app_id).await;

    let row = push_attempts::insert_attempt(
        &pool, &app_id, "quickbooks", "customer", "cust-stale", "update", 1, "fp-stale",
    )
    .await
    .expect("insert");

    push_attempts::transition_to_inflight(&pool, row.id)
        .await
        .expect("inflight");

    // Backdating started_at to simulate a stale inflight row
    sqlx::query("UPDATE integrations_sync_push_attempts SET started_at = NOW() - INTERVAL '2 hours' WHERE id = $1")
        .bind(row.id)
        .execute(&pool)
        .await
        .expect("backdate");

    // Threshold: anything older than 30 minutes
    let threshold = chrono::Utc::now() - chrono::Duration::minutes(30);
    let stale = push_attempts::list_stale_inflight(&pool, threshold, 10)
        .await
        .expect("list stale");

    assert!(
        stale.iter().any(|r| r.id == row.id),
        "backdated inflight row must appear in stale list"
    );

    cleanup(&pool, &app_id).await;
}
