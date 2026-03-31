//! E2E Test: Notifications — invoice-issued event triggers notification record,
//! payment events propagate (bd-267l)
//!
//! ## Coverage
//! 1. invoice_issued event → scheduled_notifications row (invoice_due_soon)
//! 2. payment_succeeded event → notification row in events_outbox
//! 3. payment_failed event → scheduled_notifications row (payment_retry)
//! 4. Re-publish same event_id → no duplicate rows (idempotent)
//! 5. DLQ (failed_events) empty after clean run
//!
//! ## Pattern
//! No Docker, no mocks — real notifications DB (port 5437).
//! EventConsumer called directly with fabricated BusMessages; no NATS required.

mod common;

use common::generate_test_tenant;
use event_bus::BusMessage;
use notifications_rs::{
    consumers::EventConsumer,
    handlers::{handle_invoice_issued, handle_payment_failed, handle_payment_succeeded},
    models::{
        EnvelopeMetadata, InvoiceIssuedPayload, PaymentFailedPayload, PaymentSucceededPayload,
    },
};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

// ============================================================================
// Pool helpers
// ============================================================================

async fn get_notif_pool() -> sqlx::PgPool {
    let url = std::env::var("NOTIFICATIONS_DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://notifications_user:notifications_pass@localhost:5437/notifications_db"
            .to_string()
    });
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to notifications DB");
    sqlx::migrate!("../modules/notifications/db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run notifications migrations");
    pool
}

// ============================================================================
// Message builders
// ============================================================================

fn make_invoice_issued_msg(event_id: Uuid, tenant_id: &str, invoice_id: &str) -> BusMessage {
    // due_date is required for handle_invoice_issued to schedule a reminder
    let due_date = (chrono::Utc::now() + chrono::Duration::days(30))
        .format("%Y-%m-%d")
        .to_string();
    let envelope = serde_json::json!({
        "event_id": event_id.to_string(),
        "occurred_at": "2026-02-19T00:00:00Z",
        "tenant_id": tenant_id,
        "source_module": "ar",
        "source_version": "1.0.0",
        "payload": {
            "invoice_id": invoice_id,
            "customer_id": "cust-e2e-001",
            "amount_due_minor": 10000,
            "currency": "USD",
            "due_date": due_date
        }
    });
    BusMessage {
        subject: "ar.events.invoice.issued".to_string(),
        payload: serde_json::to_vec(&envelope).unwrap(),
        headers: None,
        reply_to: None,
    }
}

fn make_payment_succeeded_msg(
    event_id: Uuid,
    tenant_id: &str,
    payment_id: &str,
    invoice_id: &str,
) -> BusMessage {
    let envelope = serde_json::json!({
        "event_id": event_id.to_string(),
        "occurred_at": "2026-02-19T00:00:00Z",
        "tenant_id": tenant_id,
        "source_module": "payments",
        "source_version": "1.0.0",
        "payload": {
            "payment_id": payment_id,
            "invoice_id": invoice_id,
            "ar_customer_id": "cust-e2e-001",
            "amount_minor": 10000,
            "currency": "USD"
        }
    });
    BusMessage {
        subject: "payments.events.payment.succeeded".to_string(),
        payload: serde_json::to_vec(&envelope).unwrap(),
        headers: None,
        reply_to: None,
    }
}

fn make_payment_failed_msg(
    event_id: Uuid,
    tenant_id: &str,
    payment_id: &str,
    invoice_id: &str,
) -> BusMessage {
    let envelope = serde_json::json!({
        "event_id": event_id.to_string(),
        "occurred_at": "2026-02-19T00:00:00Z",
        "tenant_id": tenant_id,
        "source_module": "payments",
        "source_version": "1.0.0",
        "payload": {
            "payment_id": payment_id,
            "invoice_id": invoice_id,
            "ar_customer_id": "cust-e2e-001",
            "amount_minor": 10000,
            "currency": "USD",
            "failure_code": "card_declined"
        }
    });
    BusMessage {
        subject: "payments.events.payment.failed".to_string(),
        payload: serde_json::to_vec(&envelope).unwrap(),
        headers: None,
        reply_to: None,
    }
}

// ============================================================================
// Query helpers
// ============================================================================

async fn count_outbox(pool: &sqlx::PgPool, tenant_id: &str, subject: &str) -> i64 {
    let row: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM events_outbox WHERE tenant_id = $1 AND subject = $2")
            .bind(tenant_id)
            .bind(subject)
            .fetch_one(pool)
            .await
            .unwrap_or((0,));
    row.0
}

async fn count_processed(pool: &sqlx::PgPool, event_id: Uuid) -> i64 {
    let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM processed_events WHERE event_id = $1")
        .bind(event_id)
        .fetch_one(pool)
        .await
        .unwrap_or((0,));
    row.0
}

async fn count_dlq(pool: &sqlx::PgPool, tenant_id: &str) -> i64 {
    let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM failed_events WHERE tenant_id = $1")
        .bind(tenant_id)
        .fetch_one(pool)
        .await
        .unwrap_or((0,));
    row.0
}

async fn count_scheduled(pool: &sqlx::PgPool, tenant_id: &str, template_key: &str) -> i64 {
    let row: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM scheduled_notifications WHERE tenant_id = $1 AND template_key = $2",
    )
    .bind(tenant_id)
    .bind(template_key)
    .fetch_one(pool)
    .await
    .unwrap_or((0,));
    row.0
}

// ============================================================================
// Cleanup
// ============================================================================

async fn cleanup(pool: &sqlx::PgPool, tenant_id: &str) {
    for q in [
        "DELETE FROM events_outbox WHERE tenant_id = $1",
        "DELETE FROM processed_events WHERE tenant_id = $1",
        "DELETE FROM failed_events WHERE tenant_id = $1",
        "DELETE FROM scheduled_notifications WHERE tenant_id = $1",
    ] {
        sqlx::query(q).bind(tenant_id).execute(pool).await.ok();
    }
}

// ============================================================================
// Drivers — feed a BusMessage through EventConsumer + handler
// ============================================================================

async fn drive_invoice_issued(pool: &sqlx::PgPool, msg: &BusMessage) {
    let pool_c = pool.clone();
    let bytes = msg.payload.clone();
    let consumer = EventConsumer::new(pool.clone());
    consumer
        .process_idempotent(msg, move |payload: InvoiceIssuedPayload| async move {
            let envelope: serde_json::Value = serde_json::from_slice(&bytes)?;
            let event_id = Uuid::parse_str(envelope["event_id"].as_str().unwrap())?;
            let tenant_id = envelope["tenant_id"].as_str().unwrap().to_string();
            let metadata = EnvelopeMetadata {
                event_id,
                tenant_id,
                correlation_id: None,
            };
            handle_invoice_issued(&pool_c, payload, metadata).await
        })
        .await
        .expect("drive_invoice_issued");
}

async fn drive_payment_succeeded(pool: &sqlx::PgPool, msg: &BusMessage) {
    let pool_c = pool.clone();
    let bytes = msg.payload.clone();
    let consumer = EventConsumer::new(pool.clone());
    consumer
        .process_idempotent(msg, move |payload: PaymentSucceededPayload| async move {
            let envelope: serde_json::Value = serde_json::from_slice(&bytes)?;
            let event_id = Uuid::parse_str(envelope["event_id"].as_str().unwrap())?;
            let tenant_id = envelope["tenant_id"].as_str().unwrap().to_string();
            let metadata = EnvelopeMetadata {
                event_id,
                tenant_id,
                correlation_id: None,
            };
            handle_payment_succeeded(&pool_c, payload, metadata).await
        })
        .await
        .expect("drive_payment_succeeded");
}

async fn drive_payment_failed(pool: &sqlx::PgPool, msg: &BusMessage) {
    let pool_c = pool.clone();
    let bytes = msg.payload.clone();
    let consumer = EventConsumer::new(pool.clone());
    consumer
        .process_idempotent(msg, move |payload: PaymentFailedPayload| async move {
            let envelope: serde_json::Value = serde_json::from_slice(&bytes)?;
            let event_id = Uuid::parse_str(envelope["event_id"].as_str().unwrap())?;
            let tenant_id = envelope["tenant_id"].as_str().unwrap().to_string();
            let metadata = EnvelopeMetadata {
                event_id,
                tenant_id,
                correlation_id: None,
            };
            handle_payment_failed(&pool_c, payload, metadata).await
        })
        .await
        .expect("drive_payment_failed");
}

// ============================================================================
// Test 1: invoice_issued → notification row in events_outbox
// ============================================================================

#[tokio::test]
#[serial]
async fn invoice_issued_creates_notification_row() {
    let pool = get_notif_pool().await;
    let tenant_id = generate_test_tenant();
    let event_id = Uuid::new_v4();
    let invoice_id = format!("inv-{}", Uuid::new_v4());

    let msg = make_invoice_issued_msg(event_id, &tenant_id, &invoice_id);
    drive_invoice_issued(&pool, &msg).await;

    // invoice_issued schedules an invoice_due_soon reminder (not outbox)
    let scheduled_count = count_scheduled(&pool, &tenant_id, "invoice_due_soon").await;
    assert_eq!(
        scheduled_count, 1,
        "Expected 1 invoice_due_soon row in scheduled_notifications after invoice_issued, got {scheduled_count}"
    );

    let processed_count = count_processed(&pool, event_id).await;
    assert_eq!(
        processed_count, 1,
        "Expected 1 processed_events row for invoice_issued event_id, got {processed_count}"
    );

    cleanup(&pool, &tenant_id).await;
}

// ============================================================================
// Test 2: payment_succeeded → notification row in events_outbox
// ============================================================================

#[tokio::test]
#[serial]
async fn payment_succeeded_creates_notification_row() {
    let pool = get_notif_pool().await;
    let tenant_id = generate_test_tenant();
    let event_id = Uuid::new_v4();

    let msg = make_payment_succeeded_msg(
        event_id,
        &tenant_id,
        &format!("pay-{}", Uuid::new_v4()),
        &format!("inv-{}", Uuid::new_v4()),
    );
    drive_payment_succeeded(&pool, &msg).await;

    let outbox_count = count_outbox(&pool, &tenant_id, "notifications.delivery.succeeded").await;
    assert_eq!(
        outbox_count, 1,
        "Expected 1 delivery.succeeded row after payment_succeeded, got {outbox_count}"
    );

    let processed_count = count_processed(&pool, event_id).await;
    assert_eq!(
        processed_count, 1,
        "Expected 1 processed_events row for payment_succeeded event_id, got {processed_count}"
    );

    cleanup(&pool, &tenant_id).await;
}

// ============================================================================
// Test 3: payment_failed → notification row in events_outbox
// ============================================================================

#[tokio::test]
#[serial]
async fn payment_failed_creates_notification_row() {
    let pool = get_notif_pool().await;
    let tenant_id = generate_test_tenant();
    let event_id = Uuid::new_v4();

    let msg = make_payment_failed_msg(
        event_id,
        &tenant_id,
        &format!("pay-{}", Uuid::new_v4()),
        &format!("inv-{}", Uuid::new_v4()),
    );
    drive_payment_failed(&pool, &msg).await;

    // payment_failed schedules a payment_retry reminder (not outbox)
    let scheduled_count = count_scheduled(&pool, &tenant_id, "payment_retry").await;
    assert_eq!(
        scheduled_count, 1,
        "Expected 1 payment_retry row in scheduled_notifications after payment_failed, got {scheduled_count}"
    );

    let processed_count = count_processed(&pool, event_id).await;
    assert_eq!(
        processed_count, 1,
        "Expected 1 processed_events row for payment_failed event_id, got {processed_count}"
    );

    cleanup(&pool, &tenant_id).await;
}

// ============================================================================
// Test 4: Idempotency — same event_id published twice → no duplicate rows
// ============================================================================

#[tokio::test]
#[serial]
async fn idempotent_on_republish() {
    let pool = get_notif_pool().await;
    let tenant_id = generate_test_tenant();
    let event_id = Uuid::new_v4();
    let invoice_id = format!("inv-{}", Uuid::new_v4());

    // First publish
    let msg = make_invoice_issued_msg(event_id, &tenant_id, &invoice_id);
    drive_invoice_issued(&pool, &msg).await;

    // Second publish — same event_id, must be deduplicated
    let msg2 = make_invoice_issued_msg(event_id, &tenant_id, &invoice_id);
    drive_invoice_issued(&pool, &msg2).await;

    // invoice_issued schedules to scheduled_notifications, not events_outbox
    let scheduled_count = count_scheduled(&pool, &tenant_id, "invoice_due_soon").await;
    assert_eq!(
        scheduled_count, 1,
        "Expected exactly 1 scheduled row after duplicate publish (got {scheduled_count})"
    );

    let processed_count = count_processed(&pool, event_id).await;
    assert_eq!(
        processed_count, 1,
        "Expected exactly 1 processed_events row after duplicate publish (got {processed_count})"
    );

    let dlq_count = count_dlq(&pool, &tenant_id).await;
    assert_eq!(
        dlq_count, 0,
        "DLQ must be empty after clean idempotency run (got {dlq_count})"
    );

    cleanup(&pool, &tenant_id).await;
}

// ============================================================================
// Test 5: DLQ empty after all three event types process cleanly
// ============================================================================

#[tokio::test]
#[serial]
async fn dlq_empty_after_clean_run() {
    let pool = get_notif_pool().await;
    let tenant_id = generate_test_tenant();

    let invoice_id = format!("inv-{}", Uuid::new_v4());
    let payment_id = format!("pay-{}", Uuid::new_v4());

    drive_invoice_issued(
        &pool,
        &make_invoice_issued_msg(Uuid::new_v4(), &tenant_id, &invoice_id),
    )
    .await;

    drive_payment_succeeded(
        &pool,
        &make_payment_succeeded_msg(Uuid::new_v4(), &tenant_id, &payment_id, &invoice_id),
    )
    .await;

    drive_payment_failed(
        &pool,
        &make_payment_failed_msg(
            Uuid::new_v4(),
            &tenant_id,
            &format!("pay-fail-{}", Uuid::new_v4()),
            &invoice_id,
        ),
    )
    .await;

    let dlq_count = count_dlq(&pool, &tenant_id).await;
    assert_eq!(
        dlq_count, 0,
        "DLQ must be empty after all three event types process cleanly (got {dlq_count})"
    );

    // Only payment_succeeded writes to events_outbox; invoice_issued and
    // payment_failed schedule reminders to scheduled_notifications
    let outbox_count = count_outbox(&pool, &tenant_id, "notifications.delivery.succeeded").await;
    assert_eq!(
        outbox_count, 1,
        "Expected 1 delivery.succeeded row (from payment_succeeded), got {outbox_count}"
    );

    let sched_invoice = count_scheduled(&pool, &tenant_id, "invoice_due_soon").await;
    assert_eq!(
        sched_invoice, 1,
        "Expected 1 invoice_due_soon scheduled row, got {sched_invoice}"
    );

    let sched_retry = count_scheduled(&pool, &tenant_id, "payment_retry").await;
    assert_eq!(
        sched_retry, 1,
        "Expected 1 payment_retry scheduled row, got {sched_retry}"
    );

    cleanup(&pool, &tenant_id).await;
}
