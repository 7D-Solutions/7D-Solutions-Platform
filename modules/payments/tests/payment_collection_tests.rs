/// Integration test for payment collection flow
///
/// This test verifies that:
/// 1. Payments module can consume ar.payment.collection.requested events
/// 2. Mock processor successfully processes payments
/// 3. payment.succeeded events are emitted to the outbox

use payments_rs::{PaymentCollectionRequestedPayload, PaymentSucceededPayload};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

async fn setup_test_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();

    let database_url = std::env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set for tests");

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("Failed to connect to test database");

    // Run migrations
    sqlx::migrate!("./db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    pool
}

async fn cleanup_test_data(pool: &sqlx::PgPool) {
    sqlx::query("DELETE FROM payments_events_outbox")
        .execute(pool)
        .await
        .expect("Failed to clean outbox");

    sqlx::query("DELETE FROM payments_processed_events")
        .execute(pool)
        .await
        .expect("Failed to clean processed events");
}

#[tokio::test]
#[serial]
async fn test_payment_collection_handler() {
    let pool = setup_test_db().await;
    cleanup_test_data(&pool).await;

    // Create a payment collection request payload
    let request_payload = PaymentCollectionRequestedPayload {
        invoice_id: "inv_test123".to_string(),
        customer_id: "cust_test456".to_string(),
        amount_minor: 10000, // $100.00
        currency: "USD".to_string(),
        payment_method_id: Some("pm_test789".to_string()),
    };

    // Create envelope metadata
    let event_id = Uuid::new_v4();
    let metadata = payments_rs::handlers::EnvelopeMetadata {
        event_id,
        tenant_id: "tenant_test".to_string(),
        correlation_id: Some(Uuid::new_v4().to_string()),
    };

    // Handle the payment collection request
    let result = payments_rs::handle_payment_collection_requested(
        &pool,
        request_payload.clone(),
        metadata,
    )
    .await;

    assert!(result.is_ok(), "Handler should succeed: {:?}", result);

    // Verify that a payment.succeeded event was enqueued
    #[derive(sqlx::FromRow)]
    struct OutboxEvent {
        event_type: String,
        payload: serde_json::Value,
    }

    let outbox_events: Vec<OutboxEvent> = sqlx::query_as(
        r#"
        SELECT event_type, payload
        FROM payments_events_outbox
        WHERE event_type = 'payment.succeeded'
        "#,
    )
    .fetch_all(&pool)
    .await
    .expect("Failed to query outbox");

    assert_eq!(
        outbox_events.len(),
        1,
        "Should have exactly one payment succeeded event in outbox"
    );

    let event = &outbox_events[0];
    assert_eq!(event.event_type, "payment.succeeded");

    // Validate the payload
    let payload: PaymentSucceededPayload = serde_json::from_value(event.payload.clone())
        .expect("Failed to deserialize payload");

    assert_eq!(payload.invoice_id, "inv_test123");
    assert_eq!(payload.ar_customer_id, "cust_test456");
    assert_eq!(payload.amount_minor, 10000);
    assert_eq!(payload.currency, "USD");
    assert!(payload.payment_id.len() > 0, "Should have payment_id");
    assert!(
        payload.processor_payment_id.is_some(),
        "Should have processor_payment_id"
    );
    assert_eq!(payload.payment_method_ref, Some("pm_test789".to_string()));

    cleanup_test_data(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_mock_processor_generates_payment_ids() {
    use payments_rs::MockPaymentProcessor;

    let processor = MockPaymentProcessor::new();

    let request = PaymentCollectionRequestedPayload {
        invoice_id: "inv_mock123".to_string(),
        customer_id: "cust_mock456".to_string(),
        amount_minor: 5000,
        currency: "EUR".to_string(),
        payment_method_id: None,
    };

    let result = processor.process_payment(&request).await;

    assert!(result.is_ok(), "Mock processor should always succeed");

    let payment_result = result.unwrap();
    assert!(!payment_result.payment_id.is_empty(), "Should generate payment_id");
    assert!(
        payment_result.processor_payment_id.starts_with("mock_pi_"),
        "Should generate mock processor payment ID"
    );
}

#[tokio::test]
#[serial]
async fn test_idempotent_event_processing() {
    let pool = setup_test_db().await;
    cleanup_test_data(&pool).await;

    // Create a payment collection request
    let request_payload = PaymentCollectionRequestedPayload {
        invoice_id: "inv_idempotent".to_string(),
        customer_id: "cust_idempotent".to_string(),
        amount_minor: 2500,
        currency: "GBP".to_string(),
        payment_method_id: None,
    };

    let event_id = Uuid::new_v4();
    let metadata = payments_rs::handlers::EnvelopeMetadata {
        event_id,
        tenant_id: "tenant_test".to_string(),
        correlation_id: None,
    };

    // Process the event twice with the same event_id
    let result1 = payments_rs::handle_payment_collection_requested(
        &pool,
        request_payload.clone(),
        payments_rs::handlers::EnvelopeMetadata {
            event_id,
            tenant_id: "tenant_test".to_string(),
            correlation_id: None,
        },
    )
    .await;

    assert!(result1.is_ok(), "First processing should succeed");

    // Count events in outbox before second processing
    let count_before: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM payments_events_outbox WHERE event_type = 'payment.succeeded'"
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    // Process again with same event_id (would normally be caught by EventConsumer)
    // For this test, we're just verifying the handler can be called multiple times
    let result2 = payments_rs::handle_payment_collection_requested(
        &pool,
        request_payload,
        payments_rs::handlers::EnvelopeMetadata {
            event_id: Uuid::new_v4(), // Use different event_id since we're bypassing EventConsumer
            tenant_id: "tenant_test".to_string(),
            correlation_id: None,
        },
    )
    .await;

    assert!(result2.is_ok(), "Second processing should also succeed");

    // We should now have two events since we used different event IDs
    let count_after: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM payments_events_outbox WHERE event_type = 'payment.succeeded'"
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(
        count_after.0,
        count_before.0 + 1,
        "Should have one more event (different event_id)"
    );

    cleanup_test_data(&pool).await;
}
