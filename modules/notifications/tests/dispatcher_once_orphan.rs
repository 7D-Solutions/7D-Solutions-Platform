/// Integration test: dispatch_once orphan recovery.
///
/// Inserts a row directly with status='claimed' and last_attempt_at 10 minutes
/// ago (older than the 5-minute orphan threshold).  dispatch_once resets it to
/// 'pending'.  Because deliver_at is set 1 hour in the future, the row is NOT
/// re-claimed in the same cycle, leaving it as 'pending'.
use std::sync::Arc;

use chrono::Utc;
use sqlx::PgPool;

use notifications_rs::scheduled::{dispatch_once, LoggingSender};

const DEFAULT_DB_URL: &str =
    "postgresql://notifications_user:notifications_pass@localhost:5437/notifications_db";

async fn get_pool() -> PgPool {
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| DEFAULT_DB_URL.to_string());
    let pool = PgPool::connect(&url)
        .await
        .expect("Failed to connect to notifications test DB");
    sqlx::migrate!("./db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");
    pool
}

#[tokio::test]
async fn test_dispatch_once_orphan_recovery() {
    let pool = get_pool().await;

    // Insert directly with status='claimed' and a stale last_attempt_at (10 min ago).
    // deliver_at is 1 hour in the future so the row won't be re-claimed after reset.
    let stale_attempt = Utc::now() - chrono::Duration::minutes(10);
    let future_deliver = Utc::now() + chrono::Duration::hours(1);
    let recipient_ref = format!("test-orphan:{}", uuid::Uuid::new_v4());

    let id: uuid::Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO scheduled_notifications
            (recipient_ref, channel, template_key, payload_json, deliver_at, status, last_attempt_at)
        VALUES ($1, 'email', 'test_orphan_tpl', '{}', $2, 'claimed', $3)
        RETURNING id
        "#,
    )
    .bind(&recipient_ref)
    .bind(future_deliver)
    .bind(stale_attempt)
    .fetch_one(&pool)
    .await
    .expect("insert orphan row failed");

    // dispatch_once: reset_orphaned_claims runs first, resetting our row to 'pending'.
    // claim_due_batch does NOT pick it up (deliver_at is in the future).
    let sender = Arc::new(LoggingSender);
    let result = dispatch_once(&pool, sender)
        .await
        .expect("dispatch_once failed");

    assert!(
        result.reset_count >= 1,
        "expected at least 1 orphan reset, got {}",
        result.reset_count
    );

    // Our specific row should now be 'pending'.
    let (status,): (String,) =
        sqlx::query_as("SELECT status FROM scheduled_notifications WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .expect("row not found");

    assert_eq!(
        status, "pending",
        "orphaned 'claimed' row should be reset to 'pending', got '{}'",
        status
    );
}
