/// Integration tests: escalation rules for notifications.
///
/// Tests cover: E2E escalation trigger, multi-level chains, acknowledged-before-timeout,
/// tenant isolation, idempotency, and outbox event verification.
use chrono::Utc;
use notifications_rs::escalation::{
    acknowledge_notification, create_escalation_rule, poll_and_escalate, InsertEscalationRule,
};
use notifications_rs::scheduled::{dispatch_once, insert_pending, LoggingSender, RetryPolicy};
use serial_test::serial;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

const DEFAULT_DB_URL: &str =
    "postgresql://notifications_user:notifications_pass@localhost:5437/notifications_db";

/// The template key must be one registered in the template engine.
/// We use "invoice_due_soon" which expects {invoice_id, amount, due_date}.
const TEMPLATE_KEY: &str = "invoice_due_soon";

fn valid_payload() -> serde_json::Value {
    serde_json::json!({
        "invoice_id": format!("INV-{}", Uuid::new_v4()),
        "amount": 5000,
        "due_date": "2026-04-01"
    })
}

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

/// Helper: insert a notification with a valid template and dispatch it so it
/// becomes 'sent'. Returns the notification id.
async fn insert_and_send(pool: &PgPool, tenant_id: &str) -> Uuid {
    let recipient_ref = format!("{}:user-{}", tenant_id, Uuid::new_v4());
    let due = Utc::now() - chrono::Duration::seconds(1);
    let id = insert_pending(
        pool,
        &recipient_ref,
        "email",
        TEMPLATE_KEY,
        valid_payload(),
        due,
    )
    .await
    .expect("insert pending");

    let sender: Arc<dyn notifications_rs::scheduled::NotificationSender> =
        Arc::new(LoggingSender);
    dispatch_once(pool, sender, RetryPolicy::default())
        .await
        .expect("dispatch_once");

    // Verify it's sent
    let (status,): (String,) =
        sqlx::query_as("SELECT status FROM scheduled_notifications WHERE id = $1")
            .bind(id)
            .fetch_one(pool)
            .await
            .expect("fetch status");
    assert_eq!(status, "sent", "notification should be sent");
    id
}

/// Helper: backdate sent_at to simulate time passing.
async fn backdate_sent_at(pool: &PgPool, notification_id: Uuid, secs_ago: i64) {
    let sent_at = Utc::now() - chrono::Duration::seconds(secs_ago);
    sqlx::query("UPDATE scheduled_notifications SET sent_at = $1 WHERE id = $2")
        .bind(sent_at)
        .bind(notification_id)
        .execute(pool)
        .await
        .expect("backdate sent_at");
}

// ── 1. Escalation trigger E2E ──────────────────────────────────────────

#[tokio::test]
#[serial]
async fn escalation_fires_after_timeout() {
    let pool = get_pool().await;
    let tenant = Uuid::new_v4().to_string();

    // Create escalation rule: 60s timeout, level 1
    create_escalation_rule(
        &pool,
        &InsertEscalationRule {
            tenant_id: tenant.clone(),
            source_notification_type: TEMPLATE_KEY.to_string(),
            level: 1,
            timeout_secs: 60,
            target_channel: "sms".to_string(),
            target_recipient: "manager@example.com".to_string(),
            priority: "high".to_string(),
        },
    )
    .await
    .expect("create rule");

    // Send a notification
    let notif_id = insert_and_send(&pool, &tenant).await;

    // Backdate sent_at by 120 seconds (past the 60s timeout)
    backdate_sent_at(&pool, notif_id, 120).await;

    // Run escalation poll
    let result = poll_and_escalate(&pool).await.expect("poll_and_escalate");
    assert!(
        result.escalated >= 1,
        "expected at least 1 escalation, got {}",
        result.escalated
    );

    // Verify escalation send was created with correct target
    let send = sqlx::query_as::<_, (Uuid, String, String, i32)>(
        r#"
        SELECT id, target_channel, target_recipient, level
        FROM escalation_sends
        WHERE source_notification_id = $1
        "#,
    )
    .bind(notif_id)
    .fetch_one(&pool)
    .await
    .expect("fetch escalation send");

    assert_eq!(send.1, "sms", "target channel should be sms");
    assert_eq!(
        send.2, "manager@example.com",
        "target recipient should match rule"
    );
    assert_eq!(send.3, 1, "level should be 1");
}

// ── 2. Multi-level escalation ──────────────────────────────────────────

#[tokio::test]
#[serial]
async fn multi_level_escalation_chain() {
    let pool = get_pool().await;
    let tenant = Uuid::new_v4().to_string();

    // Level 1: 30s timeout
    create_escalation_rule(
        &pool,
        &InsertEscalationRule {
            tenant_id: tenant.clone(),
            source_notification_type: TEMPLATE_KEY.to_string(),
            level: 1,
            timeout_secs: 30,
            target_channel: "email".to_string(),
            target_recipient: "supervisor@example.com".to_string(),
            priority: "high".to_string(),
        },
    )
    .await
    .expect("create level 1 rule");

    // Level 2: 120s timeout
    create_escalation_rule(
        &pool,
        &InsertEscalationRule {
            tenant_id: tenant.clone(),
            source_notification_type: TEMPLATE_KEY.to_string(),
            level: 2,
            timeout_secs: 120,
            target_channel: "sms".to_string(),
            target_recipient: "director@example.com".to_string(),
            priority: "critical".to_string(),
        },
    )
    .await
    .expect("create level 2 rule");

    let notif_id = insert_and_send(&pool, &tenant).await;

    // Backdate sent_at by 60s — past level 1 (30s) but not level 2 (120s)
    backdate_sent_at(&pool, notif_id, 60).await;

    let r1 = poll_and_escalate(&pool).await.expect("poll cycle 1");
    assert_eq!(r1.escalated, 1, "only level 1 should fire at 60s");

    // Verify level 1 send exists
    let sends: Vec<(i32,)> = sqlx::query_as(
        "SELECT level FROM escalation_sends WHERE source_notification_id = $1 ORDER BY level",
    )
    .bind(notif_id)
    .fetch_all(&pool)
    .await
    .expect("fetch sends after cycle 1");
    assert_eq!(sends.len(), 1);
    assert_eq!(sends[0].0, 1);

    // Backdate further to 180s — past level 2 (120s)
    backdate_sent_at(&pool, notif_id, 180).await;

    let r2 = poll_and_escalate(&pool).await.expect("poll cycle 2");
    assert_eq!(r2.escalated, 1, "level 2 should fire at 180s");

    // Verify both levels now have sends
    let sends: Vec<(i32, String)> = sqlx::query_as(
        "SELECT level, target_recipient FROM escalation_sends WHERE source_notification_id = $1 ORDER BY level",
    )
    .bind(notif_id)
    .fetch_all(&pool)
    .await
    .expect("fetch sends after cycle 2");
    assert_eq!(sends.len(), 2, "should have 2 escalation sends");
    assert_eq!(sends[0].0, 1);
    assert_eq!(sends[0].1, "supervisor@example.com");
    assert_eq!(sends[1].0, 2);
    assert_eq!(sends[1].1, "director@example.com");
}

// ── 3. Acknowledged-before-timeout ─────────────────────────────────────

#[tokio::test]
#[serial]
async fn no_escalation_when_acknowledged_before_timeout() {
    let pool = get_pool().await;
    let tenant = Uuid::new_v4().to_string();

    create_escalation_rule(
        &pool,
        &InsertEscalationRule {
            tenant_id: tenant.clone(),
            source_notification_type: TEMPLATE_KEY.to_string(),
            level: 1,
            timeout_secs: 60,
            target_channel: "sms".to_string(),
            target_recipient: "oncall@example.com".to_string(),
            priority: "high".to_string(),
        },
    )
    .await
    .expect("create rule");

    let notif_id = insert_and_send(&pool, &tenant).await;

    // Acknowledge the notification before timeout
    let acked = acknowledge_notification(&pool, notif_id)
        .await
        .expect("acknowledge");
    assert!(acked, "should have acknowledged");

    // Backdate sent_at past the timeout
    backdate_sent_at(&pool, notif_id, 120).await;

    // Run escalation poll — should not fire because notification is acknowledged
    let result = poll_and_escalate(&pool).await.expect("poll_and_escalate");
    assert_eq!(
        result.escalated, 0,
        "no escalation should fire for acknowledged notification"
    );

    // Verify no escalation sends exist for this notification
    let sends: Vec<(Uuid,)> =
        sqlx::query_as("SELECT id FROM escalation_sends WHERE source_notification_id = $1")
            .bind(notif_id)
            .fetch_all(&pool)
            .await
            .expect("fetch sends");
    assert!(sends.is_empty(), "no sends should exist");
}

// ── 4. Tenant isolation ────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn tenant_isolation_escalation_rules() {
    let pool = get_pool().await;
    let tenant_a = Uuid::new_v4().to_string();
    let tenant_b = Uuid::new_v4().to_string();

    // Create rules for tenant A only
    create_escalation_rule(
        &pool,
        &InsertEscalationRule {
            tenant_id: tenant_a.clone(),
            source_notification_type: TEMPLATE_KEY.to_string(),
            level: 1,
            timeout_secs: 30,
            target_channel: "email".to_string(),
            target_recipient: "admin-a@example.com".to_string(),
            priority: "high".to_string(),
        },
    )
    .await
    .expect("create rule for tenant A");

    // Query rules as tenant B — should get zero results
    let rules_b =
        notifications_rs::escalation::get_rules_for_type(&pool, &tenant_b, TEMPLATE_KEY)
            .await
            .expect("get rules for tenant B");
    assert!(
        rules_b.is_empty(),
        "tenant B must not see tenant A's escalation rules"
    );

    // Query rules as tenant A — should get exactly 1
    let rules_a =
        notifications_rs::escalation::get_rules_for_type(&pool, &tenant_a, TEMPLATE_KEY)
            .await
            .expect("get rules for tenant A");
    assert_eq!(rules_a.len(), 1, "tenant A should see their own rule");

    // Send notification for tenant B (with same template) — should NOT escalate
    let notif_b = insert_and_send(&pool, &tenant_b).await;
    backdate_sent_at(&pool, notif_b, 120).await;

    let _result = poll_and_escalate(&pool).await.expect("poll_and_escalate");

    // Verify no escalation sends for tenant B's notification
    let sends_b: Vec<(Uuid,)> =
        sqlx::query_as("SELECT id FROM escalation_sends WHERE source_notification_id = $1")
            .bind(notif_b)
            .fetch_all(&pool)
            .await
            .expect("fetch sends for tenant B");
    assert!(
        sends_b.is_empty(),
        "tenant B's notification must not trigger tenant A's rules"
    );

    // Send notification for tenant A — should escalate
    let notif_a = insert_and_send(&pool, &tenant_a).await;
    backdate_sent_at(&pool, notif_a, 120).await;

    let result_a = poll_and_escalate(&pool)
        .await
        .expect("poll_and_escalate for A");
    assert!(
        result_a.escalated >= 1,
        "tenant A's notification should trigger their own rule"
    );
}

// ── 5. Idempotency ────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn idempotent_escalation_no_duplicates() {
    let pool = get_pool().await;
    let tenant = Uuid::new_v4().to_string();

    create_escalation_rule(
        &pool,
        &InsertEscalationRule {
            tenant_id: tenant.clone(),
            source_notification_type: TEMPLATE_KEY.to_string(),
            level: 1,
            timeout_secs: 30,
            target_channel: "email".to_string(),
            target_recipient: "oncall@example.com".to_string(),
            priority: "high".to_string(),
        },
    )
    .await
    .expect("create rule");

    let notif_id = insert_and_send(&pool, &tenant).await;
    backdate_sent_at(&pool, notif_id, 120).await;

    // First poll — should escalate
    let r1 = poll_and_escalate(&pool).await.expect("poll 1");
    assert_eq!(r1.escalated, 1, "first poll should create 1 escalation");

    // Second poll — should skip (already sent)
    let r2 = poll_and_escalate(&pool).await.expect("poll 2");
    assert_eq!(
        r2.escalated, 0,
        "second poll should not create duplicate escalation"
    );
    assert!(
        r2.skipped >= 1,
        "second poll should skip the already-escalated notification"
    );

    // Verify only 1 escalation send exists
    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM escalation_sends WHERE source_notification_id = $1",
    )
    .bind(notif_id)
    .fetch_one(&pool)
    .await
    .expect("count sends");
    assert_eq!(count.0, 1, "exactly 1 escalation send should exist");
}

// ── 6. Outbox event ───────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn escalation_creates_outbox_event() {
    let pool = get_pool().await;
    let tenant = Uuid::new_v4().to_string();

    create_escalation_rule(
        &pool,
        &InsertEscalationRule {
            tenant_id: tenant.clone(),
            source_notification_type: TEMPLATE_KEY.to_string(),
            level: 1,
            timeout_secs: 30,
            target_channel: "sms".to_string(),
            target_recipient: "ops@example.com".to_string(),
            priority: "critical".to_string(),
        },
    )
    .await
    .expect("create rule");

    let notif_id = insert_and_send(&pool, &tenant).await;
    backdate_sent_at(&pool, notif_id, 120).await;

    // Record count before escalation
    let (before_count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM events_outbox \
         WHERE subject = 'notifications.escalation.fired' AND tenant_id = $1",
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .expect("count before");

    poll_and_escalate(&pool).await.expect("poll_and_escalate");

    // Verify outbox event was created
    let (after_count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM events_outbox \
         WHERE subject = 'notifications.escalation.fired' AND tenant_id = $1",
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .expect("count after");

    assert_eq!(
        after_count,
        before_count + 1,
        "exactly 1 outbox event should be created"
    );

    // Verify event payload content
    let (payload,): (serde_json::Value,) = sqlx::query_as(
        r#"
        SELECT payload FROM events_outbox
        WHERE subject = 'notifications.escalation.fired' AND tenant_id = $1
        ORDER BY created_at DESC LIMIT 1
        "#,
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .expect("fetch outbox event");

    // The payload is an EventEnvelope — the actual escalation data is in .payload
    let inner = payload.get("payload").unwrap_or(&payload);
    assert_eq!(
        inner.get("target_channel").and_then(|v| v.as_str()),
        Some("sms"),
        "outbox event should have correct target_channel"
    );
    assert_eq!(
        inner.get("target_recipient").and_then(|v| v.as_str()),
        Some("ops@example.com"),
        "outbox event should have correct target_recipient"
    );
    assert_eq!(
        inner.get("tenant_id").and_then(|v| v.as_str()),
        Some(tenant.as_str()),
        "outbox event should have correct tenant_id"
    );
    assert_eq!(
        inner.get("level").and_then(|v| v.as_i64()),
        Some(1),
        "outbox event should have correct level"
    );
}
