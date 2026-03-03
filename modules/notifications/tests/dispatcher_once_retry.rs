/// Integration test: dispatch_once retry backoff.
///
/// A FailingSender defined locally simulates delivery failure.
/// After the first dispatch fails, the row is rescheduled with an advanced
/// deliver_at and retry_count=1.  A second dispatch with LoggingSender
/// succeeds and marks the row sent.
use std::sync::{
    atomic::{AtomicI32, Ordering},
    Arc,
};

use async_trait::async_trait;
use chrono::Utc;
use sqlx::PgPool;

use notifications_rs::scheduled::{
    dispatch_once, insert_pending, LoggingSender, NotificationError, NotificationSender,
    RetryPolicy, ScheduledNotification, SendReceipt,
};

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

/// Test-only sender: returns Err for the first `fail_count` calls, then Ok.
struct FailingSender {
    remaining: AtomicI32,
}

impl FailingSender {
    fn new(fail_count: i32) -> Self {
        Self {
            remaining: AtomicI32::new(fail_count),
        }
    }
}

#[async_trait]
impl NotificationSender for FailingSender {
    async fn send(&self, _notif: &ScheduledNotification) -> Result<SendReceipt, NotificationError> {
        let prev = self.remaining.fetch_sub(1, Ordering::SeqCst);
        if prev > 0 {
            Err(NotificationError::Transient("simulated failure".to_string()))
        } else {
            Ok(SendReceipt::default())
        }
    }
}

#[derive(sqlx::FromRow)]
struct RetryRow {
    status: String,
    retry_count: i32,
    deliver_at: chrono::DateTime<Utc>,
}

#[tokio::test]
async fn test_dispatch_once_retry_backoff() {
    let pool = get_pool().await;

    // Insert a due row.
    let deliver_at = Utc::now() - chrono::Duration::seconds(1);
    let recipient_ref = format!("test-retry:{}", uuid::Uuid::new_v4());

    let id = insert_pending(
        &pool,
        &recipient_ref,
        "email",
        "payment_retry",
        serde_json::json!({"invoice_id": "INV-RETRY", "payment_id": "PAY-001", "failure_code": "card_declined"}),
        deliver_at,
    )
    .await
    .expect("insert_pending failed");

    // First dispatch: FailingSender fails once.
    // Back-off for retry_count=0: (0+1)*5 = 5 minutes.
    let before_retry = Utc::now();
    let failing = Arc::new(FailingSender::new(1));
    dispatch_once(&pool, failing, RetryPolicy::default())
        .await
        .expect("dispatch_once (fail) failed");

    // Assert: row rescheduled — failed (retry-eligible), retry_count=1, deliver_at advanced ~5 min.
    let row: RetryRow = sqlx::query_as(
        "SELECT status, retry_count, deliver_at FROM scheduled_notifications WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .expect("row not found after first dispatch");

    assert_eq!(
        row.status, "failed",
        "expected failed after retryable failure, got {}",
        row.status
    );
    assert_eq!(
        row.retry_count, 1,
        "expected retry_count=1, got {}",
        row.retry_count
    );

    // deliver_at should be ~5 minutes ahead; allow a 4-minute lower bound for timing slack.
    let min_expected = before_retry + chrono::Duration::minutes(4);
    assert!(
        row.deliver_at >= min_expected,
        "deliver_at {} was not advanced enough (expected >= {})",
        row.deliver_at,
        min_expected
    );

    // Reset deliver_at to the past so the next dispatch cycle picks up the row.
    sqlx::query(
        "UPDATE scheduled_notifications SET deliver_at = NOW() - INTERVAL '1 second' WHERE id = $1",
    )
    .bind(id)
    .execute(&pool)
    .await
    .expect("failed to reset deliver_at");

    // Second dispatch: LoggingSender succeeds.
    let sender = Arc::new(LoggingSender);
    dispatch_once(&pool, sender, RetryPolicy::default())
        .await
        .expect("dispatch_once (success) failed");

    // Assert: row now sent.
    let (status,): (String,) =
        sqlx::query_as("SELECT status FROM scheduled_notifications WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .expect("row not found after second dispatch");

    assert_eq!(status, "sent", "expected sent after retry, got {}", status);
}
