/// Integration test: replay safety via processed_events idempotency gate.
///
/// Calls process_idempotent() twice with the same event_id.  The second call
/// is short-circuited by the processed_events gate, so handle_invoice_issued
/// is invoked exactly once, resulting in exactly 1 row in scheduled_notifications.
use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use event_bus::BusMessage;
use notifications_rs::{
    consumers::EventConsumer,
    handlers::handle_invoice_issued,
    models::{EnvelopeMetadata, InvoiceIssuedPayload},
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

#[tokio::test]
async fn test_replay_safety_exactly_one_scheduled_row() {
    let pool = get_pool().await;

    let event_id = Uuid::new_v4();
    let tenant_id = Uuid::new_v4().to_string();
    let customer_id = Uuid::new_v4().to_string();
    let invoice_id = Uuid::new_v4().to_string();
    // Use a due_date 30 days out so handle_invoice_issued inserts a reminder row.
    let due_date = (Utc::now() + chrono::Duration::days(30))
        .format("%Y-%m-%d")
        .to_string();

    // Build a minimal but valid event envelope.
    let envelope_bytes = serde_json::to_vec(&serde_json::json!({
        "event_id": event_id.to_string(),
        "tenant_id": &tenant_id,
        "source_module": "ar",
        "source_version": "1.0.0",
        "occurred_at": Utc::now().to_rfc3339(),
        "payload": {
            "invoice_id": &invoice_id,
            "customer_id": &customer_id,
            "amount_due_minor": 5000,
            "currency": "USD",
            "due_date": &due_date,
        }
    }))
    .unwrap();

    let msg = BusMessage::new("ar.events.invoice.issued".to_string(), envelope_bytes);
    let consumer = EventConsumer::new(pool.clone());

    // First call: handler runs, inserts 1 scheduled_notifications row.
    {
        let pool2 = pool.clone();
        let tenant_id2 = tenant_id.clone();
        consumer
            .process_idempotent::<InvoiceIssuedPayload, _, _>(&msg, move |payload: InvoiceIssuedPayload| {
                let pool2 = pool2.clone();
                let tenant_id2 = tenant_id2.clone();
                async move {
                    let metadata = EnvelopeMetadata {
                        event_id,
                        tenant_id: tenant_id2,
                        correlation_id: None,
                    };
                    handle_invoice_issued(&pool2, payload, metadata).await
                }
            })
            .await
            .expect("first process_idempotent call failed");
    }

    // Second call with the same event_id: idempotency gate short-circuits,
    // handler is never invoked.
    {
        let pool2 = pool.clone();
        let tenant_id2 = tenant_id.clone();
        consumer
            .process_idempotent::<InvoiceIssuedPayload, _, _>(&msg, move |payload: InvoiceIssuedPayload| {
                let pool2 = pool2.clone();
                let tenant_id2 = tenant_id2.clone();
                async move {
                    let metadata = EnvelopeMetadata {
                        event_id,
                        tenant_id: tenant_id2,
                        correlation_id: None,
                    };
                    handle_invoice_issued(&pool2, payload, metadata).await
                }
            })
            .await
            .expect("second process_idempotent call failed");
    }

    // Assert exactly 1 scheduled_notifications row for this recipient.
    let recipient_ref = format!("{}:{}", tenant_id, customer_id);
    let (count,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM scheduled_notifications WHERE recipient_ref = $1")
            .bind(&recipient_ref)
            .fetch_one(&pool)
            .await
            .expect("count query failed");

    assert_eq!(
        count, 1,
        "expected exactly 1 scheduled row, got {} — idempotency gate should block the duplicate",
        count
    );
}
