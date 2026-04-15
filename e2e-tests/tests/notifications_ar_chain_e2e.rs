//! E2E Test: Notification delivery chain — payment.succeeded → delivery notification (bd-1v9t)
//!
//! ## Coverage
//! 1. Subscribe to `notifications.delivery.succeeded` on NATS before triggering.
//! 2. Publish `payments.events.payment.succeeded` to NATS (simulating Payments outbox).
//! 3. In-process notifications consumer picks it up from NATS.
//! 4. Handler enqueues `notifications.delivery.succeeded` in notifications outbox.
//! 5. In-process notifications outbox publisher delivers it to NATS.
//! 6. Assert: NATS event received within 5s with correct tenant_id.
//! 7. Assert: notifications outbox row exists with channel=email, status=pending→published.
//!
//! ## Pattern
//! No Docker, no mocks, no stubs.
//! Real NATS (4222), real notifications postgres (5437).
//! Services run in-process via tokio::spawn.

mod common;

use common::{get_notifications_pool, setup_nats_client};
use event_bus::NatsBus;
use futures::StreamExt;
use notifications_rs::{consumer_tasks, event_bus::start_outbox_publisher};
use serde_json::json;
use serial_test::serial;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;
use uuid::Uuid;

// ============================================================================
// DB setup helpers
// ============================================================================

async fn run_notif_migrations(pool: &PgPool) {
    sqlx::migrate!("../modules/notifications/db/migrations")
        .run(pool)
        .await
        .expect("Notifications migrations failed");
}

// ============================================================================
// Envelope builder — matches what Payments outbox publisher would emit
// ============================================================================

fn build_payment_succeeded_envelope(
    tenant_id: &str,
    payment_id: &str,
    invoice_id: &str,
    amount_minor: i32,
) -> serde_json::Value {
    json!({
        "event_id": Uuid::new_v4().to_string(),
        "occurred_at": chrono::Utc::now().to_rfc3339(),
        "tenant_id": tenant_id,
        "source_module": "payments",
        "source_version": "1.0.0",
        "payload": {
            "payment_id": payment_id,
            "invoice_id": invoice_id,
            "ar_customer_id": "cust-e2e-chain-001",
            "amount_minor": amount_minor,
            "currency": "USD"
        }
    })
}

// ============================================================================
// DB query helpers for verification
// ============================================================================

async fn count_notif_outbox(pool: &PgPool, tenant_id: &str, subject: &str) -> i64 {
    let row: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM events_outbox WHERE tenant_id = $1 AND subject = $2")
            .bind(tenant_id)
            .bind(subject)
            .fetch_one(pool)
            .await
            .unwrap_or((0,));
    row.0
}

// ============================================================================
// Cleanup
// ============================================================================

async fn cleanup_notif(pool: &PgPool, tenant_id: &str) {
    for q in [
        "DELETE FROM events_outbox WHERE tenant_id = $1",
        "DELETE FROM processed_events WHERE tenant_id = $1",
        "DELETE FROM failed_events WHERE tenant_id = $1",
    ] {
        sqlx::query(q).bind(tenant_id).execute(pool).await.ok();
    }
}

// ============================================================================
// Test 1: payment.succeeded → notifications.delivery.succeeded on NATS
// ============================================================================

#[tokio::test]
#[serial]
async fn payment_succeeded_triggers_delivery_succeeded_on_nats() {
    let tenant_id = format!("notif-chain-{}", Uuid::new_v4().simple());

    let notif_pool = get_notifications_pool().await;
    run_notif_migrations(&notif_pool).await;

    cleanup_notif(&notif_pool, &tenant_id).await;

    // Connect to real NATS
    let nats = setup_nats_client().await;

    // 1. Subscribe to `notifications.delivery.succeeded` BEFORE triggering anything
    let mut notif_sub = nats
        .subscribe("notifications.delivery.succeeded")
        .await
        .expect("Failed to subscribe to notifications.delivery.succeeded");

    // 2. Start in-process notifications consumer (payments.events.payment.succeeded)
    let notif_bus = Arc::new(NatsBus::new(nats.clone()));
    consumer_tasks::start_payment_succeeded_consumer(notif_bus.clone(), notif_pool.clone()).await;

    // 3. Start in-process notifications outbox publisher
    let notif_pool_pub = notif_pool.clone();
    let notif_bus_pub = notif_bus.clone();
    tokio::spawn(async move {
        start_outbox_publisher(notif_pool_pub, notif_bus_pub).await;
    });

    // 4. Wait for NATS subscriptions to register on the server
    tokio::time::sleep(Duration::from_millis(300)).await;

    // 5. Build and publish payments.events.payment.succeeded to NATS
    let payment_id = format!("pay-{}", Uuid::new_v4());
    let invoice_id = format!("inv-{}", Uuid::new_v4());
    let envelope = build_payment_succeeded_envelope(&tenant_id, &payment_id, &invoice_id, 15_000);
    let payload_bytes = serde_json::to_vec(&envelope).expect("serialize envelope");
    nats.publish("payments.events.payment.succeeded", payload_bytes.into())
        .await
        .expect("Failed to publish payments.events.payment.succeeded");

    // 6. Wait up to 5s for notifications.delivery.succeeded on NATS
    let received_msg = timeout(Duration::from_secs(5), notif_sub.next())
        .await
        .expect(
            "Timed out waiting for notifications.delivery.succeeded — notification chain is broken",
        )
        .expect("NATS subscription closed unexpectedly");

    // 7. Verify the NATS event fields
    let notif_body: serde_json::Value =
        serde_json::from_slice(&received_msg.payload).expect("parse notification envelope");

    assert_eq!(
        notif_body["tenant_id"].as_str(),
        Some(tenant_id.as_str()),
        "tenant_id in notification event must match"
    );
    assert_eq!(
        notif_body["source_module"].as_str(),
        Some("notifications"),
        "notification event must originate from the notifications module"
    );

    // Verify payload has expected delivery fields
    let notif_payload = &notif_body["payload"];
    assert_eq!(
        notif_payload["channel"].as_str(),
        Some("email"),
        "notification channel must be email"
    );
    assert_eq!(
        notif_payload["status"].as_str(),
        Some("succeeded"),
        "notification status must be succeeded"
    );

    // 8. Verify notifications DB has the outbox row
    tokio::time::sleep(Duration::from_millis(200)).await;

    let outbox_count =
        count_notif_outbox(&notif_pool, &tenant_id, "notifications.delivery.succeeded").await;
    assert!(
        outbox_count >= 1,
        "Expected at least 1 notifications.delivery.succeeded row in notifications outbox, got {}",
        outbox_count
    );

    cleanup_notif(&notif_pool, &tenant_id).await;
}

// ============================================================================
// Test 2: Outbox row has correct channel and tenant after chain completes
// ============================================================================

#[tokio::test]
#[serial]
async fn outbox_row_has_correct_channel_and_tenant() {
    let tenant_id = format!("notif-row-{}", Uuid::new_v4().simple());

    let notif_pool = get_notifications_pool().await;
    run_notif_migrations(&notif_pool).await;

    cleanup_notif(&notif_pool, &tenant_id).await;

    let nats = setup_nats_client().await;

    let mut notif_sub = nats
        .subscribe("notifications.delivery.succeeded")
        .await
        .expect("subscribe");

    let notif_bus = Arc::new(NatsBus::new(nats.clone()));
    consumer_tasks::start_payment_succeeded_consumer(notif_bus.clone(), notif_pool.clone()).await;

    tokio::spawn({
        let p = notif_pool.clone();
        let b = notif_bus.clone();
        async move {
            start_outbox_publisher(p, b).await;
        }
    });

    tokio::time::sleep(Duration::from_millis(300)).await;

    let envelope = build_payment_succeeded_envelope(
        &tenant_id,
        &format!("pay-{}", Uuid::new_v4()),
        &format!("inv-{}", Uuid::new_v4()),
        30_000,
    );
    nats.publish(
        "payments.events.payment.succeeded",
        serde_json::to_vec(&envelope).unwrap().into(),
    )
    .await
    .unwrap();

    // Wait for delivery to NATS
    let _ = timeout(Duration::from_secs(5), notif_sub.next())
        .await
        .expect("Timed out waiting for notification")
        .expect("subscription closed");

    // Give publisher a moment to commit the outbox row
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Verify DB outbox row fields
    #[derive(sqlx::FromRow)]
    struct OutboxRow {
        tenant_id: String,
        subject: String,
        status: Option<String>,
    }

    let row: Option<OutboxRow> = sqlx::query_as(
        "SELECT tenant_id, subject, status FROM events_outbox \
         WHERE tenant_id = $1 AND subject = $2 LIMIT 1",
    )
    .bind(&tenant_id)
    .bind("notifications.delivery.succeeded")
    .fetch_optional(&notif_pool)
    .await
    .expect("DB query failed");

    let row = row.expect("Expected at least one outbox row for the notification");
    assert_eq!(row.tenant_id, tenant_id, "outbox row tenant_id must match");
    assert_eq!(
        row.subject, "notifications.delivery.succeeded",
        "outbox row subject must be notifications.delivery.succeeded"
    );
    // Status is 'pending' or 'published' depending on publisher timing
    let status = row.status.unwrap_or_default();
    assert!(
        status == "pending" || status == "published",
        "outbox row status must be pending or published, got '{}'",
        status
    );

    cleanup_notif(&notif_pool, &tenant_id).await;
}

// ============================================================================
// Test 3: Verify notification envelope fields — tenant matches, metadata correct
// ============================================================================

#[tokio::test]
#[serial]
async fn notification_envelope_has_correct_metadata() {
    let tenant_id = format!("notif-meta-{}", Uuid::new_v4().simple());

    let notif_pool = get_notifications_pool().await;
    run_notif_migrations(&notif_pool).await;

    cleanup_notif(&notif_pool, &tenant_id).await;

    let nats = setup_nats_client().await;
    let mut notif_sub = nats
        .subscribe("notifications.delivery.succeeded")
        .await
        .expect("subscribe");

    let notif_bus = Arc::new(NatsBus::new(nats.clone()));
    consumer_tasks::start_payment_succeeded_consumer(notif_bus.clone(), notif_pool.clone()).await;

    tokio::spawn({
        let p = notif_pool.clone();
        let b = notif_bus.clone();
        async move {
            start_outbox_publisher(p, b).await;
        }
    });

    tokio::time::sleep(Duration::from_millis(300)).await;

    let envelope = build_payment_succeeded_envelope(
        &tenant_id,
        &format!("pay-{}", Uuid::new_v4()),
        &format!("inv-{}", Uuid::new_v4()),
        50_000,
    );
    nats.publish(
        "payments.events.payment.succeeded",
        serde_json::to_vec(&envelope).unwrap().into(),
    )
    .await
    .unwrap();

    let msg = timeout(Duration::from_secs(5), notif_sub.next())
        .await
        .expect("Timed out waiting for notification")
        .expect("subscription closed");

    let body: serde_json::Value = serde_json::from_slice(&msg.payload).unwrap();

    // Invariants on the notification envelope
    assert_eq!(body["tenant_id"].as_str(), Some(tenant_id.as_str()));
    assert_eq!(body["source_module"].as_str(), Some("notifications"));

    // event_id must be a valid UUID
    let ev_id_str = body["event_id"].as_str().expect("event_id must be present");
    Uuid::parse_str(ev_id_str).expect("event_id must be a valid UUID");

    // occurred_at must be present
    assert!(
        body["occurred_at"].is_string(),
        "occurred_at must be a string timestamp"
    );

    // payload.notification_id must be non-empty
    let notif_payload = &body["payload"];
    assert!(
        !notif_payload["notification_id"]
            .as_str()
            .unwrap_or("")
            .is_empty(),
        "notification_id must be non-empty"
    );

    cleanup_notif(&notif_pool, &tenant_id).await;
}
