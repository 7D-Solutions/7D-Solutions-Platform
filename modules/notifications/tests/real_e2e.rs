use std::sync::Arc;

use axum::{extract::Path, response::IntoResponse, routing::post, Json, Router};
use chrono::Utc;
use notifications_rs::scheduled::{
    claim_due_batch, dispatch_once, insert_pending, record_delivery_attempt_and_mutate,
    AttemptApplyOutcome, HttpEmailSender, NotificationError,
};
use sqlx::PgPool;
use tokio::net::TcpListener;

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

async fn send_stub(Path(code): Path<u16>) -> impl IntoResponse {
    let status = axum::http::StatusCode::from_u16(code).unwrap();
    (status, Json(serde_json::json!({"message_id":"provider-msg-1"})))
}

async fn start_stub_server() -> String {
    let app = Router::new().route("/send/{code}", post(send_stub));
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind stub listener");
    let addr = listener.local_addr().expect("stub local_addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("stub server failed");
    });
    format!("http://{addr}")
}

#[derive(sqlx::FromRow)]
struct StatusRow {
    status: String,
    retry_count: i32,
}

#[tokio::test]
async fn e2e_http_sender_success_and_invalid_recipient_classification() {
    let pool = get_pool().await;
    let base = start_stub_server().await;

    let due = Utc::now() - chrono::Duration::seconds(1);

    // Success path.
    let success_id = insert_pending(
        &pool,
        "user-success@example.com",
        "email",
        "invoice_due_soon",
        serde_json::json!({"email":"user-success@example.com"}),
        due,
    )
    .await
    .expect("insert success pending");

    let sender: Arc<dyn notifications_rs::scheduled::NotificationSender> = Arc::new(
        HttpEmailSender::new(
            format!("{base}/send/202"),
            "no-reply@example.com".to_string(),
            None,
        ),
    );
    let result = dispatch_once(&pool, sender).await.expect("dispatch success");
    assert!(result.sent_count >= 1, "expected at least one sent notification");

    let success_row: StatusRow =
        sqlx::query_as("SELECT status, retry_count FROM scheduled_notifications WHERE id = $1")
            .bind(success_id)
            .fetch_one(&pool)
            .await
            .expect("fetch success row");
    assert_eq!(success_row.status, "sent");
    assert_eq!(success_row.retry_count, 0);

    let (attempt_status, error_class): (String, Option<String>) = sqlx::query_as(
        "SELECT status, error_class FROM notification_delivery_attempts WHERE notification_id = $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(success_id)
    .fetch_one(&pool)
    .await
    .expect("fetch success attempt");
    assert_eq!(attempt_status, "succeeded");
    assert!(error_class.is_none());

    // Failure classification path: provider 422 -> invalid_recipient -> permanent failure.
    let fail_id = insert_pending(
        &pool,
        "user-fail@example.com",
        "email",
        "payment_retry",
        serde_json::json!({"email":"user-fail@example.com"}),
        due,
    )
    .await
    .expect("insert failure pending");

    let fail_sender: Arc<dyn notifications_rs::scheduled::NotificationSender> = Arc::new(
        HttpEmailSender::new(
            format!("{base}/send/422"),
            "no-reply@example.com".to_string(),
            None,
        ),
    );
    let result = dispatch_once(&pool, fail_sender).await.expect("dispatch failure path");
    assert!(
        result.failed_count >= 1,
        "expected at least one failed notification"
    );

    let fail_row: StatusRow =
        sqlx::query_as("SELECT status, retry_count FROM scheduled_notifications WHERE id = $1")
            .bind(fail_id)
            .fetch_one(&pool)
            .await
            .expect("fetch failure row");
    assert_eq!(fail_row.status, "failed");
    assert_eq!(fail_row.retry_count, 0);

    let (attempt_status, error_class): (String, Option<String>) = sqlx::query_as(
        "SELECT status, error_class FROM notification_delivery_attempts WHERE notification_id = $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(fail_id)
    .fetch_one(&pool)
    .await
    .expect("fetch failure attempt");
    assert_eq!(attempt_status, "failed_permanent");
    assert_eq!(error_class.as_deref(), Some("invalid_recipient"));
}

#[tokio::test]
async fn e2e_idempotency_key_reuses_stored_outcome() {
    let pool = get_pool().await;
    let due = Utc::now() - chrono::Duration::seconds(1);
    let id = insert_pending(
        &pool,
        "idempotency@example.com",
        "email",
        "invoice_due_soon",
        serde_json::json!({"email":"idempotency@example.com"}),
        due,
    )
    .await
    .expect("insert pending");

    let batch = claim_due_batch(&pool, 1).await.expect("claim due");
    let notif = batch
        .into_iter()
        .find(|n| n.id == id)
        .expect("claimed notification not found");

    let key = format!("notif:{}:attempt:{}", notif.id, notif.retry_count + 1);
    let first = record_delivery_attempt_and_mutate(
        &pool,
        &notif,
        &key,
        Err(NotificationError::Transient("transient".to_string())),
    )
    .await
    .expect("record first");
    assert_eq!(first, AttemptApplyOutcome::FailedRetryable);

    let second = record_delivery_attempt_and_mutate(
        &pool,
        &notif,
        &key,
        Ok(Default::default()),
    )
    .await
    .expect("record second");
    assert_eq!(second, AttemptApplyOutcome::DuplicateStored);

    let row: StatusRow =
        sqlx::query_as("SELECT status, retry_count FROM scheduled_notifications WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .expect("fetch row");
    assert_eq!(row.status, "pending");
    assert_eq!(row.retry_count, 1);

    let (attempts,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM notification_delivery_attempts WHERE idempotency_key = $1",
    )
    .bind(key)
    .fetch_one(&pool)
    .await
    .expect("count attempts");
    assert_eq!(attempts, 1);
}
