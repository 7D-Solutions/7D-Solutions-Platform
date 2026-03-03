/// Integration test: dispatch_once delivers a due notification.
///
/// Inserts a pending row with deliver_at in the past, calls dispatch_once,
/// and asserts the row is marked sent with last_attempt_at set.
use std::sync::Arc;

use chrono::Utc;
use sqlx::PgPool;

use notifications_rs::scheduled::{dispatch_once, insert_pending, LoggingSender, RetryPolicy};

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

#[derive(sqlx::FromRow)]
struct NotifRow {
    status: String,
    last_attempt_at: Option<chrono::DateTime<Utc>>,
}

#[tokio::test]
async fn test_dispatch_once_due_send() {
    let pool = get_pool().await;

    // Insert a due row (deliver_at 1 second in the past).
    let deliver_at = Utc::now() - chrono::Duration::seconds(1);
    let recipient_ref = format!("test-tenant:{}", uuid::Uuid::new_v4());

    let id = insert_pending(
        &pool,
        &recipient_ref,
        "email",
        "invoice_due_soon",
        serde_json::json!({"invoice_id": "INV-DUE-TEST", "amount": 5000, "due_date": "2026-04-01"}),
        deliver_at,
    )
    .await
    .expect("insert_pending failed");

    // Dispatch with LoggingSender — should succeed immediately.
    let sender = Arc::new(LoggingSender);
    let result = dispatch_once(&pool, sender, RetryPolicy::default())
        .await
        .expect("dispatch_once failed");

    assert!(
        result.sent_count >= 1,
        "expected at least 1 sent, got {}",
        result.sent_count
    );

    // Verify our specific row by ID.
    let row: NotifRow =
        sqlx::query_as("SELECT status, last_attempt_at FROM scheduled_notifications WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .expect("row not found");

    assert_eq!(
        row.status, "sent",
        "expected status=sent, got {}",
        row.status
    );
    assert!(
        row.last_attempt_at.is_some(),
        "last_attempt_at should be set after successful dispatch"
    );
}
