/// Integration tests for SMS sender: all 6 required categories.
///
/// Uses real Postgres on port 5437. Test senders implement NotificationSender
/// in-process (no external SMS gateway needed). The tests verify the full
/// Guard → Mutation → Outbox atomicity pipeline for the SMS channel.
use std::sync::{
    atomic::{AtomicI32, Ordering},
    Arc,
};

use async_trait::async_trait;
use chrono::Utc;
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

use notifications_rs::scheduled::{
    dispatch_once, insert_pending, NotificationError, NotificationSender, RetryPolicy,
    ScheduledNotification, SendReceipt,
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

/// Always-succeeding SMS sender for success-path tests.
struct SuccessSmsSender;

#[async_trait]
impl NotificationSender for SuccessSmsSender {
    async fn send(&self, notif: &ScheduledNotification) -> Result<SendReceipt, NotificationError> {
        tracing::info!(id = %notif.id, channel = %notif.channel, "test SMS sent successfully");
        Ok(SendReceipt {
            provider_message_id: Some(format!("sms-msg-{}", Uuid::new_v4())),
        })
    }
}

/// Sender that returns transient errors for `fail_count` calls, then succeeds.
struct RetryableSmsSender {
    remaining: AtomicI32,
}

impl RetryableSmsSender {
    fn new(fail_count: i32) -> Self {
        Self {
            remaining: AtomicI32::new(fail_count),
        }
    }
}

#[async_trait]
impl NotificationSender for RetryableSmsSender {
    async fn send(&self, _notif: &ScheduledNotification) -> Result<SendReceipt, NotificationError> {
        let prev = self.remaining.fetch_sub(1, Ordering::SeqCst);
        if prev > 0 {
            Err(NotificationError::Transient(
                "simulated SMS transient failure".to_string(),
            ))
        } else {
            Ok(SendReceipt {
                provider_message_id: Some("sms-recovered".to_string()),
            })
        }
    }
}

/// Sender that always returns a permanent (non-retryable) error.
struct PermanentFailSmsSender;

#[async_trait]
impl NotificationSender for PermanentFailSmsSender {
    async fn send(&self, _notif: &ScheduledNotification) -> Result<SendReceipt, NotificationError> {
        Err(NotificationError::Permanent(
            "simulated permanent SMS rejection".to_string(),
        ))
    }
}

fn sms_payload() -> serde_json::Value {
    serde_json::json!({
        "phone": "+15551234567",
        "body": "Your order has shipped.",
        "invoice_id": "INV-SMS-TEST",
        "amount": 100,
        "due_date": "2026-04-01"
    })
}

// ── 1. Success path E2E ─────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn sms_success_path_e2e() {
    let pool = get_pool().await;
    let deliver_at = Utc::now() - chrono::Duration::seconds(1);
    let recipient_ref = format!("sms-tenant-ok:+1555{}", &Uuid::new_v4().to_string()[..7]);

    let id = insert_pending(
        &pool,
        &recipient_ref,
        "sms",
        "invoice_due_soon",
        sms_payload(),
        deliver_at,
    )
    .await
    .expect("insert_pending failed");

    let sender: Arc<dyn NotificationSender> = Arc::new(SuccessSmsSender);
    let result = dispatch_once(&pool, sender, RetryPolicy::default())
        .await
        .expect("dispatch_once failed");

    assert!(result.sent_count >= 1, "expected at least 1 sent");

    // Verify notification status
    let (status,): (String,) =
        sqlx::query_as("SELECT status FROM scheduled_notifications WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .expect("row not found");
    assert_eq!(status, "sent");

    // Verify delivery attempt journaled
    let (attempt_status, provider_msg_id): (String, Option<String>) = sqlx::query_as(
        "SELECT status, provider_message_id FROM notification_delivery_attempts WHERE notification_id = $1",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .expect("delivery attempt not found");
    assert_eq!(attempt_status, "succeeded");
    assert!(
        provider_msg_id.is_some(),
        "provider_message_id should be set"
    );

    // Verify outbox event emitted
    let (event_type, tenant_id): (Option<String>, String) = sqlx::query_as(
        "SELECT event_type, tenant_id FROM events_outbox WHERE payload::text LIKE $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(format!("%{}%", id))
    .fetch_one(&pool)
    .await
    .expect("outbox event not found");
    assert_eq!(
        event_type.as_deref(),
        Some("notifications.delivery.succeeded")
    );
    assert!(!tenant_id.is_empty());
}

// ── 2. Retryable failure E2E ────────────────────────────────────────

#[tokio::test]
#[serial]
async fn sms_retryable_failure_e2e() {
    let pool = get_pool().await;
    let deliver_at = Utc::now() - chrono::Duration::seconds(1);
    let recipient_ref = format!("sms-tenant-retry:+1555{}", &Uuid::new_v4().to_string()[..7]);

    let id = insert_pending(
        &pool,
        &recipient_ref,
        "sms",
        "invoice_due_soon",
        sms_payload(),
        deliver_at,
    )
    .await
    .expect("insert_pending failed");

    // Sender that fails once (transient), then would succeed
    let sender: Arc<dyn NotificationSender> = Arc::new(RetryableSmsSender::new(1));
    let result = dispatch_once(&pool, sender, RetryPolicy::default())
        .await
        .expect("dispatch_once failed");

    assert!(
        result.rescheduled_count >= 1,
        "expected at least 1 rescheduled"
    );

    // Verify status is failed (retry-eligible) with retry_count=1
    let (status, retry_count): (String, i32) =
        sqlx::query_as("SELECT status, retry_count FROM scheduled_notifications WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .expect("row not found");
    assert_eq!(status, "failed");
    assert_eq!(retry_count, 1);

    // Verify delivery attempt journaled as failed_retryable
    let (attempt_status,): (String,) = sqlx::query_as(
        "SELECT status FROM notification_delivery_attempts WHERE notification_id = $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .expect("attempt not found");
    assert_eq!(attempt_status, "failed_retryable");
}

// ── 3. Non-retryable failure E2E ────────────────────────────────────

#[tokio::test]
#[serial]
async fn sms_non_retryable_failure_e2e() {
    let pool = get_pool().await;
    let deliver_at = Utc::now() - chrono::Duration::seconds(1);
    let recipient_ref = format!("sms-tenant-perm:+1555{}", &Uuid::new_v4().to_string()[..7]);

    let id = insert_pending(
        &pool,
        &recipient_ref,
        "sms",
        "invoice_due_soon",
        sms_payload(),
        deliver_at,
    )
    .await
    .expect("insert_pending failed");

    let sender: Arc<dyn NotificationSender> = Arc::new(PermanentFailSmsSender);
    let result = dispatch_once(&pool, sender, RetryPolicy::default())
        .await
        .expect("dispatch_once failed");

    assert!(result.failed_count >= 1, "expected at least 1 failed");

    // Verify dead_lettered status
    let (status,): (String,) =
        sqlx::query_as("SELECT status FROM scheduled_notifications WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .expect("row not found");
    assert_eq!(status, "dead_lettered");

    // Verify delivery attempt journaled as failed_permanent
    let (attempt_status,): (String,) = sqlx::query_as(
        "SELECT status FROM notification_delivery_attempts WHERE notification_id = $1",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .expect("attempt not found");
    assert_eq!(attempt_status, "failed_permanent");

    // Verify outbox event type
    let (event_type,): (Option<String>,) = sqlx::query_as(
        "SELECT event_type FROM events_outbox WHERE payload::text LIKE $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(format!("%{}%", id))
    .fetch_one(&pool)
    .await
    .expect("outbox event not found");
    assert_eq!(event_type.as_deref(), Some("notifications.delivery.failed"));
}

// ── 4. Idempotency test ─────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn sms_idempotency_no_duplicate_delivery() {
    let pool = get_pool().await;
    let deliver_at = Utc::now() - chrono::Duration::seconds(1);
    let recipient_ref = format!("sms-tenant-idem:+1555{}", &Uuid::new_v4().to_string()[..7]);

    let id = insert_pending(
        &pool,
        &recipient_ref,
        "sms",
        "invoice_due_soon",
        sms_payload(),
        deliver_at,
    )
    .await
    .expect("insert_pending failed");

    // First dispatch — succeeds
    let sender: Arc<dyn NotificationSender> = Arc::new(SuccessSmsSender);
    dispatch_once(&pool, sender, RetryPolicy::default())
        .await
        .expect("dispatch_once failed");

    let (status,): (String,) =
        sqlx::query_as("SELECT status FROM scheduled_notifications WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .expect("row not found");
    assert_eq!(status, "sent");

    // Count delivery attempts before second dispatch
    let (count_before,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM notification_delivery_attempts WHERE notification_id = $1",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .expect("count failed");

    // Force the row back to pending to simulate a replay
    sqlx::query(
        "UPDATE scheduled_notifications SET status = 'pending', deliver_at = NOW() - INTERVAL '1 second' WHERE id = $1",
    )
    .bind(id)
    .execute(&pool)
    .await
    .expect("force pending failed");

    // Second dispatch — should hit idempotency guard (same idempotency_key)
    let sender2: Arc<dyn NotificationSender> = Arc::new(SuccessSmsSender);
    dispatch_once(&pool, sender2, RetryPolicy::default())
        .await
        .expect("dispatch_once (second) failed");

    // No new delivery attempt should be created
    let (count_after,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM notification_delivery_attempts WHERE notification_id = $1",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .expect("count failed");

    assert_eq!(
        count_before, count_after,
        "idempotency_key should prevent duplicate delivery attempt: before={}, after={}",
        count_before, count_after
    );
}

// ── 5. Tenant isolation test ────────────────────────────────────────

#[tokio::test]
#[serial]
async fn sms_tenant_isolation() {
    let pool = get_pool().await;
    let deliver_at = Utc::now() - chrono::Duration::seconds(1);
    let tenant_a = format!("sms-iso-a-{}", Uuid::new_v4());
    let tenant_b = format!("sms-iso-b-{}", Uuid::new_v4());

    // Insert SMS for tenant A
    let id_a = insert_pending(
        &pool,
        &format!("{}:+15551111111", tenant_a),
        "sms",
        "invoice_due_soon",
        sms_payload(),
        deliver_at,
    )
    .await
    .expect("insert A failed");

    // Dispatch tenant A's SMS
    let sender: Arc<dyn NotificationSender> = Arc::new(SuccessSmsSender);
    dispatch_once(&pool, sender, RetryPolicy::default())
        .await
        .expect("dispatch_once failed");

    // Verify tenant A can see their delivery log
    let rows_a: Vec<(Uuid,)> = sqlx::query_as(
        "SELECT da.notification_id FROM notification_delivery_attempts da \
         JOIN scheduled_notifications sn ON da.notification_id = sn.id \
         WHERE sn.tenant_id = $1",
    )
    .bind(&tenant_a)
    .fetch_all(&pool)
    .await
    .expect("tenant A query failed");
    assert!(
        rows_a.iter().any(|r| r.0 == id_a),
        "tenant A should see their own delivery attempt"
    );

    // Verify tenant B sees zero delivery logs
    let rows_b: Vec<(Uuid,)> = sqlx::query_as(
        "SELECT da.notification_id FROM notification_delivery_attempts da \
         JOIN scheduled_notifications sn ON da.notification_id = sn.id \
         WHERE sn.tenant_id = $1",
    )
    .bind(&tenant_b)
    .fetch_all(&pool)
    .await
    .expect("tenant B query failed");
    assert!(
        rows_b.is_empty(),
        "tenant B must see zero delivery logs, got {}",
        rows_b.len()
    );
}

// ── 6. Outbox event verification ────────────────────────────────────

#[tokio::test]
#[serial]
async fn sms_outbox_event_has_correct_fields() {
    let pool = get_pool().await;
    let deliver_at = Utc::now() - chrono::Duration::seconds(1);
    let tenant_tag = format!("sms-outbox-{}", Uuid::new_v4());
    let recipient_ref = format!("{}:+15559999999", tenant_tag);

    let id = insert_pending(
        &pool,
        &recipient_ref,
        "sms",
        "invoice_due_soon",
        sms_payload(),
        deliver_at,
    )
    .await
    .expect("insert_pending failed");

    let sender: Arc<dyn NotificationSender> = Arc::new(SuccessSmsSender);
    dispatch_once(&pool, sender, RetryPolicy::default())
        .await
        .expect("dispatch_once failed");

    // Fetch outbox row for this notification
    #[derive(sqlx::FromRow)]
    #[allow(dead_code)]
    struct OutboxRow {
        tenant_id: String,
        event_type: Option<String>,
        correlation_id: Option<String>,
        payload: serde_json::Value,
    }

    let row: OutboxRow = sqlx::query_as(
        "SELECT tenant_id, event_type, correlation_id, payload \
         FROM events_outbox \
         WHERE payload::text LIKE $1 \
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(format!("%{}%", id))
    .fetch_one(&pool)
    .await
    .expect("outbox row not found");

    // Correct tenant_id
    assert_eq!(row.tenant_id, tenant_tag);

    // Correct event type
    assert_eq!(
        row.event_type.as_deref(),
        Some("notifications.delivery.succeeded")
    );

    // The outbox stores the full EventEnvelope; the inner payload is at .payload
    let inner = row
        .payload
        .get("payload")
        .expect("envelope should have .payload");

    // Payload contains notification_id
    let payload_notif_id = inner.get("notification_id").and_then(|v| v.as_str());
    assert_eq!(payload_notif_id, Some(id.to_string()).as_deref());

    // Payload contains attempt_status
    let attempt_status = inner.get("attempt_status").and_then(|v| v.as_str());
    assert_eq!(attempt_status, Some("succeeded"));
}
