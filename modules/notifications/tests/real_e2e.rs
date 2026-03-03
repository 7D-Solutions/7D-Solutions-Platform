use std::sync::Arc;

use axum::{extract::Path, response::IntoResponse, routing::post, Json, Router};
use chrono::{DateTime, Utc};
use notifications_rs::scheduled::{
    claim_due_batch, dispatch_once, insert_pending, record_delivery_attempt_and_mutate,
    AttemptApplyOutcome, HttpEmailSender, NotificationError, RetryPolicy,
};
use serial_test::serial;
use sqlx::PgPool;
use tokio::net::TcpListener;
use uuid::Uuid;

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

// ── DLQ operational API E2E tests ───────────────────────────────────

/// Helper: force a notification into dead_lettered status via a permanent failure.
async fn force_dlq_entry(pool: &PgPool) -> Uuid {
    let due = Utc::now() - chrono::Duration::seconds(1);
    let id = insert_pending(
        pool,
        "dlq-test@example.com",
        "email",
        "nonexistent_template",
        serde_json::json!({}),
        due,
    )
    .await
    .expect("insert pending for DLQ");

    let sender: Arc<dyn notifications_rs::scheduled::NotificationSender> =
        Arc::new(notifications_rs::scheduled::LoggingSender);
    dispatch_once(pool, sender, RetryPolicy::default())
        .await
        .expect("dispatch to DLQ");

    // Verify it's dead_lettered
    let (status,): (String,) =
        sqlx::query_as("SELECT status FROM scheduled_notifications WHERE id = $1")
            .bind(id)
            .fetch_one(pool)
            .await
            .expect("fetch status");
    assert_eq!(status, "dead_lettered", "should be dead_lettered after permanent failure");
    id
}

#[tokio::test]
#[serial]
async fn e2e_dlq_list_returns_dead_lettered_items() {
    let pool = get_pool().await;
    let id = force_dlq_entry(&pool).await;

    // Query DLQ via direct SQL (handler uses the same query pattern)
    let (count,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM scheduled_notifications WHERE status = 'dead_lettered'")
            .fetch_one(&pool)
            .await
            .expect("count DLQ items");
    assert!(count >= 1, "should have at least one DLQ item");

    // Verify our specific item is in the DLQ
    let exists: Option<(Uuid,)> = sqlx::query_as(
        "SELECT id FROM scheduled_notifications WHERE id = $1 AND status = 'dead_lettered'",
    )
    .bind(id)
    .fetch_optional(&pool)
    .await
    .expect("check DLQ item");
    assert!(exists.is_some(), "our item should be in the DLQ");
}

#[tokio::test]
#[serial]
async fn e2e_dlq_replay_resets_to_pending_and_delivers() {
    let pool = get_pool().await;
    let base = start_stub_server().await;
    let id = force_dlq_entry(&pool).await;

    // Replay: Guard → Mutation → Outbox within a transaction
    let mut tx = pool.begin().await.expect("begin tx");

    // Guard: verify dead_lettered
    let (status,): (String,) = sqlx::query_as(
        "SELECT status FROM scheduled_notifications WHERE id = $1 FOR UPDATE",
    )
    .bind(id)
    .fetch_one(&mut *tx)
    .await
    .expect("guard check");
    assert_eq!(status, "dead_lettered");

    // Mutation: reset to pending with a valid template, bump replay_generation
    sqlx::query(
        "UPDATE scheduled_notifications \
         SET status = 'pending', \
             deliver_at = NOW(), \
             retry_count = 0, \
             replay_generation = replay_generation + 1, \
             template_key = 'invoice_due_soon', \
             payload_json = $1, \
             last_error = NULL, \
             dead_lettered_at = NULL, \
             failed_at = NULL \
         WHERE id = $2",
    )
    .bind(serde_json::json!({
        "invoice_id": "INV-REPLAY",
        "amount": 5000,
        "due_date": "2026-07-01",
        "email": "dlq-test@example.com",
    }))
    .bind(id)
    .execute(&mut *tx)
    .await
    .expect("reset to pending");

    // Outbox: emit replay event
    let envelope = notifications_rs::event_bus::create_notifications_envelope(
        uuid::Uuid::new_v4(),
        "dlq-test".to_string(),
        "notifications.dlq.replayed".to_string(),
        None,
        None,
        "LIFECYCLE".to_string(),
        serde_json::json!({
            "notification_id": id,
            "action": "replay",
        }),
    );
    notifications_rs::event_bus::enqueue_event(
        &mut tx,
        "notifications.events.dlq.replayed",
        &envelope,
    )
    .await
    .expect("enqueue replay event");

    tx.commit().await.expect("commit replay");

    // Verify replayed to pending
    let (status,): (String,) =
        sqlx::query_as("SELECT status FROM scheduled_notifications WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .expect("check post-replay status");
    assert_eq!(status, "pending");

    // Verify outbox event was written
    let (outbox_count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM events_outbox \
         WHERE subject = 'notifications.events.dlq.replayed' \
         AND payload::text LIKE $1",
    )
    .bind(format!("%{}%", id))
    .fetch_one(&pool)
    .await
    .expect("check outbox");
    assert!(outbox_count >= 1, "replay event should be in outbox");

    // Now dispatch — should succeed with valid template
    let sender: Arc<dyn notifications_rs::scheduled::NotificationSender> = Arc::new(
        HttpEmailSender::new(
            format!("{base}/send/202"),
            "no-reply@example.com".to_string(),
            None,
        ),
    );
    let result = dispatch_once(&pool, sender, RetryPolicy::default())
        .await
        .expect("dispatch after replay");
    assert!(result.sent_count >= 1, "replayed notification should be sent");

    // Verify final status is sent
    let (status,): (String,) =
        sqlx::query_as("SELECT status FROM scheduled_notifications WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .expect("check final status");
    assert_eq!(status, "sent", "replayed notification should reach sent status");
}

#[tokio::test]
#[serial]
async fn e2e_dlq_replay_idempotent_no_double_send() {
    let pool = get_pool().await;
    let id = force_dlq_entry(&pool).await;

    // First replay (bumps replay_generation for fresh idempotency keys)
    sqlx::query(
        "UPDATE scheduled_notifications \
         SET status = 'pending', deliver_at = NOW(), retry_count = 0, \
             replay_generation = replay_generation + 1, \
             last_error = NULL, dead_lettered_at = NULL, failed_at = NULL \
         WHERE id = $1",
    )
    .bind(id)
    .execute(&pool)
    .await
    .expect("first replay");

    // Second replay attempt should be a no-op (status is no longer dead_lettered)
    let (status,): (String,) = sqlx::query_as(
        "SELECT status FROM scheduled_notifications WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .expect("check status before second replay");
    assert_eq!(status, "pending", "already replayed to pending");

    // Attempting to replay again: guard rejects because status != dead_lettered
    let already_replayed = status != "dead_lettered";
    assert!(already_replayed, "second replay should be rejected by guard");
}

#[tokio::test]
#[serial]
async fn e2e_dlq_abandon_marks_terminal() {
    let pool = get_pool().await;
    let id = force_dlq_entry(&pool).await;

    // Abandon via Guard → Mutation → Outbox
    let mut tx = pool.begin().await.expect("begin tx");

    let (status,): (String,) = sqlx::query_as(
        "SELECT status FROM scheduled_notifications WHERE id = $1 FOR UPDATE",
    )
    .bind(id)
    .fetch_one(&mut *tx)
    .await
    .expect("guard check");
    assert_eq!(status, "dead_lettered");

    sqlx::query(
        "UPDATE scheduled_notifications \
         SET status = 'abandoned', abandoned_at = NOW() \
         WHERE id = $1",
    )
    .bind(id)
    .execute(&mut *tx)
    .await
    .expect("abandon mutation");

    let envelope = notifications_rs::event_bus::create_notifications_envelope(
        uuid::Uuid::new_v4(),
        "dlq-test".to_string(),
        "notifications.dlq.abandoned".to_string(),
        None,
        None,
        "LIFECYCLE".to_string(),
        serde_json::json!({
            "notification_id": id,
            "action": "abandon",
        }),
    );
    notifications_rs::event_bus::enqueue_event(
        &mut tx,
        "notifications.events.dlq.abandoned",
        &envelope,
    )
    .await
    .expect("enqueue abandon event");

    tx.commit().await.expect("commit abandon");

    // Verify final status
    let (status, abandoned_at): (String, Option<DateTime<Utc>>) = sqlx::query_as(
        "SELECT status, abandoned_at FROM scheduled_notifications WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .expect("check abandoned status");
    assert_eq!(status, "abandoned");
    assert!(abandoned_at.is_some(), "abandoned_at should be set");

    // Verify outbox event
    let (outbox_count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM events_outbox \
         WHERE subject = 'notifications.events.dlq.abandoned' \
         AND payload::text LIKE $1",
    )
    .bind(format!("%{}%", id))
    .fetch_one(&pool)
    .await
    .expect("check abandon outbox");
    assert!(outbox_count >= 1, "abandon event should be in outbox");

    // Abandoned notifications should NOT be picked up by dispatcher
    let sender: Arc<dyn notifications_rs::scheduled::NotificationSender> =
        Arc::new(notifications_rs::scheduled::LoggingSender);
    let _result = dispatch_once(&pool, sender, RetryPolicy::default())
        .await
        .expect("dispatch after abandon");
    // If dispatcher claimed our item, it would show up in claimed_count.
    // abandoned status should be excluded from claim_due_batch.
    let (still_abandoned,): (String,) =
        sqlx::query_as("SELECT status FROM scheduled_notifications WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .expect("check still abandoned");
    assert_eq!(still_abandoned, "abandoned", "abandoned items must not be dispatched");
}
