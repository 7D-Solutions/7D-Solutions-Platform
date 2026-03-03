use std::sync::Arc;

use axum::{extract::Path, response::IntoResponse, routing::post, Json, Router};
use chrono::Utc;
use notifications_rs::scheduled::{
    claim_due_batch, dispatch_once, insert_pending, record_delivery_attempt_and_mutate,
    AttemptApplyOutcome, HttpEmailSender, NotificationError, RetryPolicy,
};
use serial_test::serial;
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
#[serial]
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
        serde_json::json!({"email":"user-success@example.com", "invoice_id":"INV-S1", "amount":1000, "due_date":"2026-04-01"}),
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
    let result = dispatch_once(&pool, sender, RetryPolicy::default())
        .await
        .expect("dispatch success");
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
        serde_json::json!({"email":"user-fail@example.com", "invoice_id":"INV-F1", "payment_id":"PAY-F1", "failure_code":"card_declined"}),
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
    let result = dispatch_once(&pool, fail_sender, RetryPolicy::default())
        .await
        .expect("dispatch failure path");
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
    assert_eq!(fail_row.status, "dead_lettered");
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
#[serial]
async fn e2e_idempotency_key_reuses_stored_outcome() {
    let pool = get_pool().await;
    let due = Utc::now() - chrono::Duration::seconds(1);
    let id = insert_pending(
        &pool,
        "idempotency@example.com",
        "email",
        "invoice_due_soon",
        serde_json::json!({"email":"idempotency@example.com", "invoice_id":"INV-IDEM", "amount":500, "due_date":"2026-05-01"}),
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
        None,
        RetryPolicy::default(),
    )
    .await
    .expect("record first");
    assert_eq!(first, AttemptApplyOutcome::FailedRetryable);

    let second = record_delivery_attempt_and_mutate(
        &pool,
        &notif,
        &key,
        Ok(Default::default()),
        None,
        RetryPolicy::default(),
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
    assert_eq!(row.status, "failed");
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

// ── Template rendering E2E tests ────────────────────────────────────

#[tokio::test]
#[serial]
async fn e2e_rendered_content_persisted_on_successful_dispatch() {
    let pool = get_pool().await;
    let base = start_stub_server().await;
    let due = Utc::now() - chrono::Duration::seconds(1);

    let id = insert_pending(
        &pool,
        "render-test@example.com",
        "email",
        "invoice_due_soon",
        serde_json::json!({
            "invoice_id": "INV-RENDER-001",
            "amount": 7500,
            "due_date": "2026-06-15",
            "email": "render-test@example.com",
        }),
        due,
    )
    .await
    .expect("insert pending for render test");

    let sender: Arc<dyn notifications_rs::scheduled::NotificationSender> = Arc::new(
        HttpEmailSender::new(
            format!("{base}/send/202"),
            "no-reply@example.com".to_string(),
            None,
        ),
    );
    let result = dispatch_once(&pool, sender, RetryPolicy::default())
        .await
        .expect("dispatch render test");
    assert!(result.sent_count >= 1);

    // Verify rendered content was persisted alongside the delivery attempt.
    let (subject, body_html, body_text): (Option<String>, Option<String>, Option<String>) =
        sqlx::query_as(
            "SELECT rendered_subject, rendered_body_html, rendered_body_text \
             FROM notification_delivery_attempts WHERE notification_id = $1 \
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(id)
        .fetch_one(&pool)
        .await
        .expect("fetch rendered content");

    let subject = subject.expect("rendered_subject should not be null");
    let body_html = body_html.expect("rendered_body_html should not be null");
    let body_text = body_text.expect("rendered_body_text should not be null");

    assert!(
        subject.contains("INV-RENDER-001"),
        "subject should contain invoice_id, got: {}",
        subject
    );
    assert!(body_html.contains("<strong>INV-RENDER-001</strong>"));
    assert!(body_text.contains("7500"));
    assert!(body_text.contains("2026-06-15"));
}

#[tokio::test]
#[serial]
async fn e2e_render_failure_records_permanent_failure() {
    let pool = get_pool().await;
    let due = Utc::now() - chrono::Duration::seconds(1);

    // Insert with a template key that does NOT exist in the registry.
    let id = insert_pending(
        &pool,
        "render-fail@example.com",
        "email",
        "nonexistent_template",
        serde_json::json!({}),
        due,
    )
    .await
    .expect("insert pending for render failure");

    let sender: Arc<dyn notifications_rs::scheduled::NotificationSender> =
        Arc::new(notifications_rs::scheduled::LoggingSender);
    let result = dispatch_once(&pool, sender, RetryPolicy::default())
        .await
        .expect("dispatch render failure");
    assert!(result.failed_count >= 1, "render failure should count as failed");

    let row: StatusRow =
        sqlx::query_as("SELECT status, retry_count FROM scheduled_notifications WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .expect("fetch row");
    assert_eq!(
        row.status, "dead_lettered",
        "unknown template → DLQ terminal status"
    );

    let (attempt_status, error_class): (String, Option<String>) = sqlx::query_as(
        "SELECT status, error_class FROM notification_delivery_attempts \
         WHERE notification_id = $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .expect("fetch attempt");
    assert_eq!(attempt_status, "failed_permanent");
    assert_eq!(error_class.as_deref(), Some("render_failure"));

    // Rendered columns should be NULL on render failure.
    let (subject,): (Option<String>,) = sqlx::query_as(
        "SELECT rendered_subject FROM notification_delivery_attempts \
         WHERE notification_id = $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .expect("fetch rendered_subject");
    assert!(subject.is_none(), "rendered_subject should be null on render failure");
}

#[tokio::test]
async fn e2e_render_idempotent_across_calls() {
    // Verify that rendering the same template+payload produces identical output
    // across separate invocations — deterministic rendering guarantee.
    use notifications_rs::templates::render;

    let payload = serde_json::json!({
        "invoice_id": "INV-IDEM",
        "amount": 1234,
        "due_date": "2026-12-31",
    });

    let first = render("invoice_due_soon", &payload).expect("render first");
    let second = render("invoice_due_soon", &payload).expect("render second");
    assert_eq!(first, second, "same inputs must produce identical outputs");
}
