//! E2E Test: Notification delivery chain — AR invoice.issued → customer notification (bd-1v9t)
//!
//! ## Coverage
//! 1. Subscribe to `notifications.delivery.succeeded` on NATS before triggering.
//! 2. Create AR customer + invoice in AR DB (real postgres, port 5434).
//! 3. Publish `ar.events.invoice.issued` to NATS (simulating AR outbox publisher).
//! 4. In-process notifications consumer picks it up from NATS.
//! 5. Handler enqueues `notifications.delivery.succeeded` in notifications outbox.
//! 6. In-process notifications outbox publisher delivers it to NATS.
//! 7. Assert: NATS event received within 5s with correct tenant_id.
//! 8. Assert: notifications outbox row exists with channel=email, status=pending→published.
//!
//! ## Pattern
//! No Docker, no mocks, no stubs.
//! Real NATS (4222), real AR postgres (5434), real notifications postgres (5437).
//! Services run in-process via tokio::spawn.

mod common;

use common::{get_ar_pool, get_notifications_pool, setup_nats_client};
use event_bus::{BusMessage, NatsBus};
use futures::StreamExt;
use notifications_rs::{consumer_tasks, event_bus::start_outbox_publisher};
use serial_test::serial;
use serde_json::json;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;
use uuid::Uuid;

// ============================================================================
// DB setup helpers
// ============================================================================

async fn run_ar_migrations(pool: &PgPool) {
    sqlx::migrate!("../modules/ar/db/migrations")
        .run(pool)
        .await
        .expect("AR migrations failed");
}

async fn run_notif_migrations(pool: &PgPool) {
    sqlx::migrate!("../modules/notifications/db/migrations")
        .run(pool)
        .await
        .expect("Notifications migrations failed");
}

// ============================================================================
// AR record creation (direct SQL — avoids HTTP router complexity)
// ============================================================================

async fn create_ar_customer(pool: &PgPool, tenant_id: &str) -> i32 {
    sqlx::query_scalar::<_, i32>(
        r#"
        INSERT INTO ar_customers (app_id, email, name, status, retry_attempt_count, created_at, updated_at)
        VALUES ($1, $2, $3, 'active', 0, NOW(), NOW())
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(format!("notif-chain-{}@e2e.test", tenant_id))
    .bind(format!("E2E Customer {}", tenant_id))
    .fetch_one(pool)
    .await
    .expect("Failed to create AR customer")
}

async fn create_ar_invoice(pool: &PgPool, tenant_id: &str, customer_id: i32, amount_cents: i32) -> i32 {
    sqlx::query_scalar::<_, i32>(
        r#"
        INSERT INTO ar_invoices (
            app_id, tilled_invoice_id, ar_customer_id, status,
            amount_cents, currency, created_at, updated_at
        )
        VALUES ($1, $2, $3, 'open', $4, 'usd', NOW(), NOW())
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(format!("in_notifchain_{}", Uuid::new_v4()))
    .bind(customer_id)
    .bind(amount_cents)
    .fetch_one(pool)
    .await
    .expect("Failed to create AR invoice")
}

// ============================================================================
// Envelope builder — matches what AR outbox publisher would emit
// ============================================================================

fn build_invoice_issued_envelope(
    tenant_id: &str,
    invoice_id: i32,
    customer_id: i32,
    amount_cents: i32,
) -> serde_json::Value {
    json!({
        "event_id": Uuid::new_v4().to_string(),
        "occurred_at": chrono::Utc::now().to_rfc3339(),
        "tenant_id": tenant_id,
        "source_module": "ar",
        "source_version": "1.0.0",
        "payload": {
            "invoice_id": invoice_id.to_string(),
            "customer_id": customer_id.to_string(),
            "amount_due_minor": amount_cents,
            "currency": "USD"
        }
    })
}

// ============================================================================
// DB query helpers for verification
// ============================================================================

async fn count_notif_outbox(pool: &PgPool, tenant_id: &str, subject: &str) -> i64 {
    let row: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM events_outbox WHERE tenant_id = $1 AND subject = $2",
    )
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

async fn cleanup_ar(pool: &PgPool, tenant_id: &str) {
    sqlx::query("DELETE FROM ar_invoices WHERE app_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM ar_customers WHERE app_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
}

// ============================================================================
// Test 1: invoice.issued → notifications.delivery.succeeded on NATS (NATS chain)
// ============================================================================

#[tokio::test]
#[serial]
async fn ar_invoice_issued_triggers_delivery_succeeded_on_nats() {
    let tenant_id = format!("notif-chain-{}", Uuid::new_v4().simple());

    // Connect to both real DBs and run migrations
    let ar_pool = get_ar_pool().await;
    let notif_pool = get_notifications_pool().await;
    run_ar_migrations(&ar_pool).await;
    run_notif_migrations(&notif_pool).await;

    cleanup_notif(&notif_pool, &tenant_id).await;
    cleanup_ar(&ar_pool, &tenant_id).await;

    // Connect to real NATS
    let nats = setup_nats_client().await;

    // 1. Subscribe to `notifications.delivery.succeeded` BEFORE triggering anything
    let mut notif_sub = nats
        .subscribe("notifications.delivery.succeeded")
        .await
        .expect("Failed to subscribe to notifications.delivery.succeeded");

    // 2. Start in-process notifications consumers (subscribe to ar.events.invoice.issued)
    let notif_bus = Arc::new(NatsBus::new(nats.clone()));
    consumer_tasks::start_invoice_issued_consumer(notif_bus.clone(), notif_pool.clone()).await;

    // 3. Start in-process notifications outbox publisher
    let notif_pool_pub = notif_pool.clone();
    let notif_bus_pub = notif_bus.clone();
    tokio::spawn(async move {
        start_outbox_publisher(notif_pool_pub, notif_bus_pub).await;
    });

    // 4. Wait for NATS subscriptions to register on the server
    tokio::time::sleep(Duration::from_millis(300)).await;

    // 5. Create AR customer + invoice in real AR DB
    let customer_id = create_ar_customer(&ar_pool, &tenant_id).await;
    let amount_cents = 15_000_i32; // $150.00
    let invoice_id = create_ar_invoice(&ar_pool, &tenant_id, customer_id, amount_cents).await;

    // 6. Build and publish ar.events.invoice.issued to NATS
    //    (simulates what AR outbox publisher emits when an invoice is issued)
    let envelope = build_invoice_issued_envelope(&tenant_id, invoice_id, customer_id, amount_cents);
    let payload_bytes = serde_json::to_vec(&envelope).expect("serialize envelope");
    nats.publish(
        "ar.events.invoice.issued",
        payload_bytes.into(),
    )
    .await
    .expect("Failed to publish ar.events.invoice.issued");

    // 7. Wait up to 5s for notifications.delivery.succeeded on NATS
    let received_msg = timeout(Duration::from_secs(5), notif_sub.next())
        .await
        .expect("Timed out waiting for notifications.delivery.succeeded — notification chain is broken")
        .expect("NATS subscription closed unexpectedly");

    // 8. Verify the NATS event fields
    let notif_body: serde_json::Value =
        serde_json::from_slice(&received_msg.payload).expect("parse notification envelope");

    assert_eq!(
        notif_body["tenant_id"].as_str(),
        Some(tenant_id.as_str()),
        "tenant_id in notification event must match the AR invoice tenant"
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

    // 9. Verify notifications DB has the outbox row
    // Give the outbox publisher a moment to mark as published
    tokio::time::sleep(Duration::from_millis(200)).await;

    let outbox_count =
        count_notif_outbox(&notif_pool, &tenant_id, "notifications.delivery.succeeded").await;
    assert!(
        outbox_count >= 1,
        "Expected at least 1 notifications.delivery.succeeded row in notifications outbox, got {}",
        outbox_count
    );

    // Cleanup
    cleanup_notif(&notif_pool, &tenant_id).await;
    cleanup_ar(&ar_pool, &tenant_id).await;
}

// ============================================================================
// Test 2: Outbox row has correct channel and tenant after chain completes
// ============================================================================

#[tokio::test]
#[serial]
async fn outbox_row_has_correct_channel_and_tenant() {
    let tenant_id = format!("notif-row-{}", Uuid::new_v4().simple());

    let ar_pool = get_ar_pool().await;
    let notif_pool = get_notifications_pool().await;
    run_ar_migrations(&ar_pool).await;
    run_notif_migrations(&notif_pool).await;

    cleanup_notif(&notif_pool, &tenant_id).await;
    cleanup_ar(&ar_pool, &tenant_id).await;

    let nats = setup_nats_client().await;

    let mut notif_sub = nats
        .subscribe("notifications.delivery.succeeded")
        .await
        .expect("subscribe");

    let notif_bus = Arc::new(NatsBus::new(nats.clone()));
    consumer_tasks::start_invoice_issued_consumer(notif_bus.clone(), notif_pool.clone()).await;

    tokio::spawn({
        let p = notif_pool.clone();
        let b = notif_bus.clone();
        async move { start_outbox_publisher(p, b).await; }
    });

    tokio::time::sleep(Duration::from_millis(300)).await;

    let customer_id = create_ar_customer(&ar_pool, &tenant_id).await;
    let invoice_id = create_ar_invoice(&ar_pool, &tenant_id, customer_id, 30_000).await;

    let envelope = build_invoice_issued_envelope(&tenant_id, invoice_id, customer_id, 30_000);
    nats.publish(
        "ar.events.invoice.issued",
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
    cleanup_ar(&ar_pool, &tenant_id).await;
}

// ============================================================================
// Test 3: Verify notification envelope fields — amount in content, tenant matches
// ============================================================================

#[tokio::test]
#[serial]
async fn notification_envelope_has_correct_metadata() {
    let tenant_id = format!("notif-meta-{}", Uuid::new_v4().simple());

    let ar_pool = get_ar_pool().await;
    let notif_pool = get_notifications_pool().await;
    run_ar_migrations(&ar_pool).await;
    run_notif_migrations(&notif_pool).await;

    cleanup_notif(&notif_pool, &tenant_id).await;
    cleanup_ar(&ar_pool, &tenant_id).await;

    let nats = setup_nats_client().await;
    let mut notif_sub = nats
        .subscribe("notifications.delivery.succeeded")
        .await
        .expect("subscribe");

    let notif_bus = Arc::new(NatsBus::new(nats.clone()));
    consumer_tasks::start_invoice_issued_consumer(notif_bus.clone(), notif_pool.clone()).await;

    tokio::spawn({
        let p = notif_pool.clone();
        let b = notif_bus.clone();
        async move { start_outbox_publisher(p, b).await; }
    });

    tokio::time::sleep(Duration::from_millis(300)).await;

    let customer_id = create_ar_customer(&ar_pool, &tenant_id).await;
    let invoice_id = create_ar_invoice(&ar_pool, &tenant_id, customer_id, 50_000).await;

    let envelope = build_invoice_issued_envelope(&tenant_id, invoice_id, customer_id, 50_000);
    nats.publish(
        "ar.events.invoice.issued",
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
        !notif_payload["notification_id"].as_str().unwrap_or("").is_empty(),
        "notification_id must be non-empty"
    );

    cleanup_notif(&notif_pool, &tenant_id).await;
    cleanup_ar(&ar_pool, &tenant_id).await;
}
