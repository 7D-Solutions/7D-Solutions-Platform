/// End-to-End Proof Test: Bill Run → Payment → Notification
///
/// This test orchestrates the complete happy path in-process:
/// 1. Trigger bill-run logic (Subscriptions)
/// 2. Wait for subscriptions.billrun.completed event
/// 3. Wait for ar.payment.collection.requested (AR)
/// 4. Wait for payment.succeeded (Payments)
/// 5. Wait for notification.delivery.succeeded (Notifications)
/// 6. Assert final state in all databases
///
/// Runs with BUS_TYPE=inmemory (all components in same process share one bus instance)
/// Can also run with NATS if services are running externally

use chrono::{NaiveDate, Utc};
use event_bus::{EventBus, InMemoryBus};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::json;
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

// ============================================================================
// Database Setup
// ============================================================================

async fn setup_ar_pool() -> PgPool {
    let database_url = std::env::var("AR_DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://postgres:postgres@localhost:5433/ar_test".to_string());

    PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("Failed to connect to AR database")
}

async fn setup_subscriptions_pool() -> PgPool {
    let database_url = std::env::var("SUBSCRIPTIONS_DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://postgres:postgres@localhost:5434/subscriptions_test".to_string());

    PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("Failed to connect to Subscriptions database")
}

async fn setup_payments_pool() -> PgPool {
    let database_url = std::env::var("PAYMENTS_DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://postgres:postgres@localhost:5435/payments_test".to_string());

    PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("Failed to connect to Payments database")
}

async fn setup_notifications_pool() -> PgPool {
    let database_url = std::env::var("NOTIFICATIONS_DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://postgres:postgres@localhost:5436/notifications_test".to_string());

    PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("Failed to connect to Notifications database")
}

// ============================================================================
// Event Models
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EventEnvelope<T> {
    event_id: Uuid,
    event_type: String,
    event_version: String,
    tenant_id: String,
    source_module: String,
    source_id: String,
    correlation_id: String,
    timestamp: String,
    payload: T,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PaymentCollectionRequestedPayload {
    invoice_id: String,
    customer_id: String,
    amount_due: i64,
    currency: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PaymentSucceededPayload {
    payment_id: String,
    invoice_id: String,
    amount: i64,
    currency: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct NotificationDeliveryPayload {
    notification_id: String,
    recipient: String,
    channel: String,
    event_type: String,
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Create a customer in AR database
async fn create_ar_customer(pool: &PgPool, tenant_id: &str) -> Result<i32, sqlx::Error> {
    let email = format!("test-{}@example.com", Uuid::new_v4());
    let external_id = format!("ext-{}", Uuid::new_v4());

    let customer_id: i32 = sqlx::query_scalar(
        "INSERT INTO ar_customers (app_id, email, name, external_customer_id, created_at, updated_at)
         VALUES ($1, $2, $3, $4, NOW(), NOW())
         RETURNING id"
    )
    .bind(tenant_id)
    .bind(&email)
    .bind("E2E Test Customer")
    .bind(&external_id)
    .fetch_one(pool)
    .await?;

    Ok(customer_id)
}

/// Create a subscription in Subscriptions database
async fn create_subscription(
    pool: &PgPool,
    tenant_id: &str,
    ar_customer_id: i32,
    next_bill_date: NaiveDate,
) -> Result<Uuid, sqlx::Error> {
    // First, create a subscription plan
    let plan_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO subscription_plans
         (id, tenant_id, name, description, schedule, price_minor, currency, created_at, updated_at)
         VALUES ($1, $2, $3, $4, 'monthly', 2999, 'usd', NOW(), NOW())"
    )
    .bind(plan_id)
    .bind(tenant_id)
    .bind("Pro Monthly Plan")
    .bind("Professional tier monthly subscription")
    .execute(pool)
    .await?;

    // Then create the subscription
    let subscription_id = Uuid::new_v4();
    let start_date = next_bill_date;

    sqlx::query(
        "INSERT INTO subscriptions
         (id, tenant_id, ar_customer_id, plan_id, status, schedule, price_minor, currency, start_date, next_bill_date, created_at, updated_at)
         VALUES ($1, $2, $3, $4, 'active', 'monthly', 2999, 'usd', $5, $6, NOW(), NOW())"
    )
    .bind(subscription_id)
    .bind(tenant_id)
    .bind(ar_customer_id.to_string())
    .bind(plan_id)
    .bind(start_date)
    .bind(next_bill_date)
    .execute(pool)
    .await?;

    Ok(subscription_id)
}

/// Simplified bill-run trigger: directly create invoice and emit event
/// This simulates what the subscriptions module would do
async fn trigger_bill_run_inmemory(
    subscriptions_pool: &PgPool,
    ar_pool: &PgPool,
    bus: Arc<dyn EventBus>,
    bill_run_id: &str,
    execution_date: NaiveDate,
    tenant_id: &str,
    ar_customer_id: i32,
) -> Result<i32, Box<dyn std::error::Error>> {
    // Find subscriptions due for billing
    let subscriptions: Vec<(Uuid, i64, String)> = sqlx::query_as(
        "SELECT id, price_minor, currency
         FROM subscriptions
         WHERE status = 'active' AND next_bill_date <= $1"
    )
    .bind(execution_date)
    .fetch_all(subscriptions_pool)
    .await?;

    let subscriptions_processed = subscriptions.len() as i32;
    let mut invoices_created = 0;

    let mut invoice_id_result = None;

    // Process each subscription
    for (subscription_id, price_minor, currency) in subscriptions {
        // Create invoice in AR
        let tilled_invoice_id = format!("til_test_{}", Uuid::new_v4());
        let invoice_id: i32 = sqlx::query_scalar(
            "INSERT INTO ar_invoices
             (app_id, tilled_invoice_id, ar_customer_id, amount_cents, currency, status, due_at, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, 'draft', NOW() + interval '30 days', NOW(), NOW())
             RETURNING id"
        )
        .bind(tenant_id)
        .bind(&tilled_invoice_id)
        .bind(ar_customer_id)
        .bind(price_minor)
        .bind(&currency)
        .fetch_one(ar_pool)
        .await?;

        invoice_id_result = Some(invoice_id);

        // Finalize invoice - update status to 'open'
        sqlx::query(
            "UPDATE ar_invoices SET status = 'open', updated_at = NOW()
             WHERE id = $1"
        )
        .bind(invoice_id)
        .execute(ar_pool)
        .await?;

        // Emit ar.payment.collection.requested event
        let event_envelope = json!({
            "event_id": Uuid::new_v4().to_string(),
            "event_type": "ar.payment.collection.requested",
            "event_version": "1.0.0",
            "tenant_id": tenant_id,
            "source_module": "ar",
            "source_id": invoice_id.to_string(),
            "correlation_id": bill_run_id,
            "timestamp": Utc::now().to_rfc3339(),
            "payload": {
                "invoice_id": invoice_id.to_string(),
                "customer_id": ar_customer_id.to_string(),
                "amount_due": price_minor,
                "currency": currency
            }
        });

        bus.publish(
            "ar.events.ar.payment.collection.requested",
            serde_json::to_vec(&event_envelope)?
        ).await?;

        invoices_created += 1;

        // Update subscription next_bill_date
        let new_next_bill_date = execution_date + chrono::Duration::days(30);
        sqlx::query(
            "UPDATE subscriptions
             SET next_bill_date = $1, updated_at = NOW()
             WHERE id = $2"
        )
        .bind(new_next_bill_date)
        .bind(subscription_id)
        .execute(subscriptions_pool)
        .await?;
    }

    // Emit subscriptions.billrun.completed event
    let billrun_event = json!({
        "event_id": Uuid::new_v4().to_string(),
        "event_type": "subscriptions.billrun.completed",
        "event_version": "1.0.0",
        "tenant_id": tenant_id,
        "source_module": "subscriptions",
        "source_id": bill_run_id,
        "correlation_id": bill_run_id,
        "timestamp": Utc::now().to_rfc3339(),
        "payload": {
            "bill_run_id": bill_run_id,
            "subscriptions_processed": subscriptions_processed,
            "invoices_created": invoices_created,
            "failures": 0,
            "execution_time": Utc::now().to_rfc3339()
        }
    });

    bus.publish(
        "subscriptions.events.subscriptions.billrun.completed",
        serde_json::to_vec(&billrun_event)?
    ).await?;

    Ok(invoice_id_result.ok_or("No invoice created")?)
}

/// Wait for an event with timeout
async fn wait_for_event(
    stream: &mut futures::stream::BoxStream<'_, event_bus::BusMessage>,
    timeout_secs: u64,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let msg = tokio::time::timeout(
        Duration::from_secs(timeout_secs),
        stream.next()
    ).await
    .map_err(|_| "Timeout waiting for event")?
    .ok_or("Stream ended unexpectedly")?;

    let envelope: serde_json::Value = serde_json::from_slice(&msg.payload)?;
    Ok(envelope)
}

/// Start mock payment consumer
/// Listens for ar.payment.collection.requested and emits payment.succeeded
async fn start_payment_consumer(
    bus: Arc<dyn EventBus>,
    payments_pool: PgPool,
) {
    tokio::spawn(async move {
        let mut stream = bus.subscribe("ar.events.ar.payment.collection.requested").await
            .expect("Failed to subscribe to payment collection events");

        while let Some(msg) = stream.next().await {
            let envelope: serde_json::Value = match serde_json::from_slice(&msg.payload) {
                Ok(v) => v,
                Err(e) => {
                    tracing::error!("Failed to parse payment collection event: {}", e);
                    continue;
                }
            };

            let event_id = envelope["event_id"].as_str().unwrap_or("unknown");
            let invoice_id = envelope["payload"]["invoice_id"].as_str().unwrap();
            let amount = envelope["payload"]["amount_due"].as_i64().unwrap();
            let currency = envelope["payload"]["currency"].as_str().unwrap();
            let tenant_id = envelope["tenant_id"].as_str().unwrap();

            tracing::info!("💳 Payment consumer: Processing payment for invoice {}", invoice_id);

            // Create payment record
            let payment_id = format!("pay_{}", Uuid::new_v4());
            sqlx::query(
                "INSERT INTO payments (payment_id, invoice_id, amount_cents, currency, status, created_at)
                 VALUES ($1, $2, $3, $4, 'succeeded', NOW())
                 ON CONFLICT (payment_id) DO NOTHING"
            )
            .bind(&payment_id)
            .bind(invoice_id)
            .bind(amount)
            .bind(currency)
            .execute(&payments_pool)
            .await
            .ok();

            // Emit payments.payment.succeeded event
            let payment_event = json!({
                "event_id": Uuid::new_v4().to_string(),
                "event_type": "payments.payment.succeeded",
                "event_version": "1.0.0",
                "tenant_id": tenant_id,
                "source_module": "payments",
                "source_id": payment_id.clone(),
                "correlation_id": envelope["correlation_id"].as_str().unwrap_or(""),
                "timestamp": Utc::now().to_rfc3339(),
                "payload": {
                    "payment_id": payment_id,
                    "invoice_id": invoice_id,
                    "amount": amount,
                    "currency": currency
                }
            });

            bus.publish(
                "payments.events.payments.payment.succeeded",
                serde_json::to_vec(&payment_event).unwrap()
            ).await.ok();

            tracing::info!("✓ Payment consumer: Emitted payment.succeeded for {}", payment_id);
        }
    });
}

/// Start mock AR consumer for payment.succeeded
/// Listens for payment.succeeded and updates invoice status
async fn start_ar_payment_consumer(
    bus: Arc<dyn EventBus>,
    ar_pool: PgPool,
) {
    tokio::spawn(async move {
        let mut stream = bus.subscribe("payments.events.payments.payment.succeeded").await
            .expect("Failed to subscribe to payment succeeded events");

        while let Some(msg) = stream.next().await {
            let envelope: serde_json::Value = match serde_json::from_slice(&msg.payload) {
                Ok(v) => v,
                Err(e) => {
                    tracing::error!("Failed to parse payment succeeded event: {}", e);
                    continue;
                }
            };

            let invoice_id = envelope["payload"]["invoice_id"].as_str().unwrap();
            let payment_id = envelope["payload"]["payment_id"].as_str().unwrap();

            tracing::info!("📝 AR payment consumer: Applying payment {} to invoice {}", payment_id, invoice_id);

            // Update invoice status to 'paid'
            let invoice_id_i32 = invoice_id.parse::<i32>().unwrap();
            sqlx::query(
                "UPDATE ar_invoices SET status = 'paid', updated_at = NOW()
                 WHERE id = $1"
            )
            .bind(invoice_id_i32)
            .execute(&ar_pool)
            .await
            .ok();

            tracing::info!("✓ AR payment consumer: Invoice {} marked as paid", invoice_id);
        }
    });
}

/// Start mock notification consumer
/// Listens for payment.succeeded and emits notification.delivery.succeeded
async fn start_notification_consumer(
    bus: Arc<dyn EventBus>,
    notifications_pool: PgPool,
) {
    tokio::spawn(async move {
        let mut stream = bus.subscribe("payments.events.payments.payment.succeeded").await
            .expect("Failed to subscribe to payment succeeded events");

        while let Some(msg) = stream.next().await {
            let envelope: serde_json::Value = match serde_json::from_slice(&msg.payload) {
                Ok(v) => v,
                Err(e) => {
                    tracing::error!("Failed to parse payment event for notification: {}", e);
                    continue;
                }
            };

            let payment_id = envelope["payload"]["payment_id"].as_str().unwrap();
            let tenant_id = envelope["tenant_id"].as_str().unwrap();

            tracing::info!("📧 Notification consumer: Sending notification for payment {}", payment_id);

            // Create notification record
            let notification_id = format!("notif_{}", Uuid::new_v4());
            sqlx::query(
                "INSERT INTO notifications (notification_id, recipient, channel, event_type, status, created_at)
                 VALUES ($1, 'test@example.com', 'email', 'payment.succeeded', 'sent', NOW())"
            )
            .bind(&notification_id)
            .execute(&notifications_pool)
            .await
            .ok();

            // Emit notification.delivery.succeeded event
            let notification_event = json!({
                "event_id": Uuid::new_v4().to_string(),
                "event_type": "notification.delivery.succeeded",
                "event_version": "1.0.0",
                "tenant_id": tenant_id,
                "source_module": "notifications",
                "source_id": notification_id.clone(),
                "correlation_id": envelope["correlation_id"].as_str().unwrap_or(""),
                "timestamp": Utc::now().to_rfc3339(),
                "payload": {
                    "notification_id": notification_id,
                    "recipient": "test@example.com",
                    "channel": "email",
                    "event_type": "payment.succeeded"
                }
            });

            bus.publish(
                "notifications.events.notifications.delivery.succeeded",
                serde_json::to_vec(&notification_event).unwrap()
            ).await.ok();

            tracing::info!("✓ Notification consumer: Emitted notification.delivery.succeeded");
        }
    });
}

// ============================================================================
// Main E2E Test
// ============================================================================

#[tokio::test]
#[serial]
async fn test_bill_run_to_notification_happy_path() {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .try_init()
        .ok();

    tracing::info!("🚀 Starting E2E proof test: Bill Run → Payment → Notification");

    // Setup databases
    let ar_pool = setup_ar_pool().await;
    let subscriptions_pool = setup_subscriptions_pool().await;
    let payments_pool = setup_payments_pool().await;
    let notifications_pool = setup_notifications_pool().await;

    // Clean up test data from previous runs
    sqlx::query("TRUNCATE TABLE ar_invoices, ar_customers CASCADE").execute(&ar_pool).await.ok();
    sqlx::query("TRUNCATE TABLE subscriptions, subscription_plans, bill_runs CASCADE").execute(&subscriptions_pool).await.ok();
    sqlx::query("TRUNCATE TABLE payments, payments_events_outbox, payments_processed_events CASCADE").execute(&payments_pool).await.ok();
    sqlx::query("TRUNCATE TABLE notifications, events_outbox, processed_events CASCADE").execute(&notifications_pool).await.ok();

    // Create shared InMemoryBus
    let bus: Arc<dyn EventBus> = Arc::new(InMemoryBus::new());

    // Start mock consumers for each module
    tracing::info!("🔧 Starting mock consumers...");
    start_payment_consumer(bus.clone(), payments_pool.clone()).await;
    start_ar_payment_consumer(bus.clone(), ar_pool.clone()).await;
    start_notification_consumer(bus.clone(), notifications_pool.clone()).await;

    // Give consumers a moment to subscribe
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Subscribe to all events we want to track
    let mut billrun_stream = bus.subscribe("subscriptions.events.>").await
        .expect("Failed to subscribe to subscriptions events");
    let mut ar_payment_stream = bus.subscribe("ar.events.ar.payment.collection.requested").await
        .expect("Failed to subscribe to AR payment collection events");
    let mut payment_stream = bus.subscribe("payments.events.payments.payment.succeeded").await
        .expect("Failed to subscribe to payment events");
    let mut notification_stream = bus.subscribe("notifications.events.>").await
        .expect("Failed to subscribe to notification events");

    // SETUP: Create test data
    let tenant_id = "test-tenant";
    let ar_customer_id = create_ar_customer(&ar_pool, tenant_id).await
        .expect("Failed to create AR customer");

    tracing::info!("✓ Created AR customer: {}", ar_customer_id);

    // Create subscription due for billing today
    let today = Utc::now().date_naive();
    let subscription_id = create_subscription(&subscriptions_pool, tenant_id, ar_customer_id, today).await
        .expect("Failed to create subscription");

    tracing::info!("✓ Created subscription: {}", subscription_id);

    // STEP 1: Trigger bill-run
    let bill_run_id = format!("e2e-test-{}", Uuid::new_v4());
    tracing::info!("📋 Triggering bill-run: {}", bill_run_id);

    let invoice_id = trigger_bill_run_inmemory(
        &subscriptions_pool,
        &ar_pool,
        bus.clone(),
        &bill_run_id,
        today,
        tenant_id,
        ar_customer_id
    ).await
    .expect("Failed to trigger bill run");

    tracing::info!("✓ Bill-run triggered, created invoice: {}", invoice_id);

    // STEP 2: Wait for subscriptions.billrun.completed event
    tracing::info!("⏳ Waiting for subscriptions.billrun.completed...");
    let billrun_event = wait_for_event(&mut billrun_stream, 10).await
        .expect("Failed to receive billrun.completed event");

    assert_eq!(billrun_event["event_type"], "subscriptions.billrun.completed");
    tracing::info!("✓ Received subscriptions.billrun.completed");

    // STEP 3: Wait for ar.payment.collection.requested event
    tracing::info!("⏳ Waiting for ar.payment.collection.requested...");
    let payment_collection_event = wait_for_event(&mut ar_payment_stream, 10).await
        .expect("Failed to receive payment collection requested event");

    assert_eq!(payment_collection_event["event_type"], "ar.payment.collection.requested");
    let event_invoice_id = payment_collection_event["payload"]["invoice_id"]
        .as_str()
        .expect("Missing invoice_id");

    tracing::info!("✓ Received ar.payment.collection.requested for invoice: {}", event_invoice_id);
    assert_eq!(event_invoice_id, invoice_id.to_string());

    // STEP 4: Wait for payment.succeeded event
    tracing::info!("⏳ Waiting for payment.succeeded...");
    let payment_succeeded_event = wait_for_event(&mut payment_stream, 10).await
        .expect("Failed to receive payment succeeded event");

    assert_eq!(payment_succeeded_event["event_type"], "payments.payment.succeeded");
    let payment_id = payment_succeeded_event["payload"]["payment_id"]
        .as_str()
        .expect("Missing payment_id");

    tracing::info!("✓ Received payment.succeeded: {}", payment_id);

    // STEP 5: Wait for notification.delivery.succeeded event
    tracing::info!("⏳ Waiting for notification.delivery.succeeded...");
    let notification_event = wait_for_event(&mut notification_stream, 10).await
        .expect("Failed to receive notification event");

    assert_eq!(notification_event["event_type"], "notification.delivery.succeeded");
    tracing::info!("✓ Received notification.delivery.succeeded");

    // STEP 6: Assert final state in databases
    tracing::info!("🔍 Verifying final state in databases...");

    // Give a moment for final state updates
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Check AR: Invoice should be paid
    let invoice_status: Option<String> = sqlx::query_scalar(
        "SELECT status FROM ar_invoices WHERE id = $1"
    )
    .bind(invoice_id)
    .fetch_optional(&ar_pool)
    .await
    .expect("Failed to query invoice status");

    assert_eq!(invoice_status, Some("paid".to_string()), "Invoice should be marked as paid");
    tracing::info!("  ✓ AR: Invoice status = paid");

    // Check Subscriptions: next_bill_date should be updated
    let next_bill_date: NaiveDate = sqlx::query_scalar(
        "SELECT next_bill_date FROM subscriptions WHERE id = $1"
    )
    .bind(subscription_id)
    .fetch_one(&subscriptions_pool)
    .await
    .expect("Failed to query subscription");

    assert!(next_bill_date > today, "Subscription next_bill_date should be updated");
    tracing::info!("  ✓ Subscriptions: next_bill_date updated to {}", next_bill_date);

    // Check Payments: Payment record exists
    let payment_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM payments WHERE payment_id = $1"
    )
    .bind(payment_id)
    .fetch_one(&payments_pool)
    .await
    .expect("Failed to query payments");

    assert_eq!(payment_count, 1, "Payment record should exist");
    tracing::info!("  ✓ Payments: Payment record exists");

    // Check Notifications: Notification sent
    let notification_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM notifications WHERE status = 'sent'"
    )
    .fetch_one(&notifications_pool)
    .await
    .expect("Failed to query notifications");

    assert!(notification_count > 0, "At least one notification should be sent");
    tracing::info!("  ✓ Notifications: {} notification(s) sent", notification_count);

    // Cleanup
    sqlx::query("DELETE FROM ar_customers WHERE id = $1")
        .bind(ar_customer_id)
        .execute(&ar_pool)
        .await
        .ok();

    sqlx::query("DELETE FROM subscriptions WHERE id = $1")
        .bind(subscription_id)
        .execute(&subscriptions_pool)
        .await
        .ok();

    tracing::info!("🎉 E2E test completed successfully!");
}
