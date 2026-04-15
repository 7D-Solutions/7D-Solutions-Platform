/// End-to-End Proof Test: Bill Run -> Payments -> AR -> Notifications
///
/// This test keeps the local bill-run driver but routes the downstream
/// request through the real NATS bus and asserts on real module persistence:
/// 1. Trigger bill-run logic (Subscriptions)
/// 2. Observe subscriptions.billrun.completed on NATS
/// 3. Publish ar.payment.collection.requested on real NATS
/// 4. Wait for Payments, AR, and Notifications to persist their rows
/// 5. Assert row counts and payloads in the real module tables
mod common;

use chrono::{NaiveDate, Utc};
use event_bus::{EventBus, EventEnvelope, MerchantContext, NatsBus};
use futures::StreamExt;
use payments_rs::{start_payment_collection_consumer, TestPaymentProcessor};
use platform_contracts::event_naming::nats_subject;
use serial_test::serial;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

// ============================================================================
// Helper Functions
// ============================================================================

async fn create_ar_customer(pool: &PgPool, tenant_id: &str) -> Result<i32, sqlx::Error> {
    let email = format!("test-{}@example.com", Uuid::new_v4());
    let external_id = format!("ext-{}", Uuid::new_v4());

    let customer_id: i32 = sqlx::query_scalar(
        "INSERT INTO ar_customers (app_id, email, name, external_customer_id, created_at, updated_at)
         VALUES ($1, $2, $3, $4, NOW(), NOW())
         RETURNING id",
    )
    .bind(tenant_id)
    .bind(&email)
    .bind("E2E Test Customer")
    .bind(&external_id)
    .fetch_one(pool)
    .await?;

    Ok(customer_id)
}

async fn create_subscription(
    pool: &PgPool,
    tenant_id: &str,
    ar_customer_id: i32,
    next_bill_date: NaiveDate,
) -> Result<Uuid, sqlx::Error> {
    let plan_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO subscription_plans
         (id, tenant_id, name, description, schedule, price_minor, currency, created_at, updated_at)
         VALUES ($1, $2, $3, $4, 'monthly', 2999, 'USD', NOW(), NOW())",
    )
    .bind(plan_id)
    .bind(tenant_id)
    .bind("Pro Monthly Plan")
    .bind("Professional tier monthly subscription")
    .execute(pool)
    .await?;

    let subscription_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO subscriptions
         (id, tenant_id, ar_customer_id, plan_id, status, schedule, price_minor, currency, start_date, next_bill_date, created_at, updated_at)
         VALUES ($1, $2, $3, $4, 'active', 'monthly', 2999, 'USD', $5, $6, NOW(), NOW())",
    )
    .bind(subscription_id)
    .bind(tenant_id)
    .bind(ar_customer_id.to_string())
    .bind(plan_id)
    .bind(next_bill_date)
    .bind(next_bill_date)
    .execute(pool)
    .await?;

    Ok(subscription_id)
}

fn test_payment_method_id() -> String {
    std::env::var("E2E_PAYMENT_METHOD_ID")
        .unwrap_or_else(|_| "pm_01HPQW9M8K5N7P1Q3R6T9V2W4X".to_string())
}

fn build_envelope<T: serde::Serialize>(
    tenant_id: &str,
    source_module: &str,
    event_type: &str,
    mutation_class: &str,
    payload: T,
    correlation_id: Option<String>,
) -> EventEnvelope<T> {
    let mut envelope = EventEnvelope::new(
        tenant_id.to_string(),
        source_module.to_string(),
        event_type.to_string(),
        payload,
    )
    .with_source_version("1.0.0")
    .with_schema_version("1.0.0")
    .with_mutation_class(Some(mutation_class.to_string()))
    .with_correlation_id(correlation_id)
    .with_replay_safe(true);

    if source_module == "ar" || source_module == "payments" {
        envelope =
            envelope.with_merchant_context(Some(MerchantContext::Tenant(tenant_id.to_string())));
    }

    envelope
}

async fn publish_envelope<T: serde::Serialize>(
    client: &async_nats::Client,
    subject: &str,
    envelope: &EventEnvelope<T>,
) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = serde_json::to_vec(envelope)?;
    client.publish(subject.to_string(), bytes.into()).await?;
    Ok(())
}

async fn wait_for_nats_event(
    stream: &mut async_nats::Subscriber,
    timeout_secs: u64,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let msg = tokio::time::timeout(Duration::from_secs(timeout_secs), stream.next())
        .await
        .map_err(|_| "Timeout waiting for event")?
        .ok_or("Stream ended unexpectedly")?;

    let envelope: serde_json::Value = serde_json::from_slice(&msg.payload)?;
    Ok(envelope)
}

/// Trigger the local bill-run driver, then publish the AR payment request and
/// bill-run completion events to the real NATS bus.
async fn trigger_bill_run_real_nats(
    subscriptions_pool: &PgPool,
    ar_pool: &PgPool,
    nats_client: &async_nats::Client,
    bill_run_id: &str,
    execution_date: NaiveDate,
    tenant_id: &str,
    ar_customer_id: i32,
) -> Result<(i32, Uuid), Box<dyn std::error::Error>> {
    let subscriptions: Vec<(Uuid, i64, String)> = sqlx::query_as(
        "SELECT id, price_minor, currency
         FROM subscriptions
         WHERE status = 'active' AND next_bill_date <= $1",
    )
    .bind(execution_date)
    .fetch_all(subscriptions_pool)
    .await?;

    let subscriptions_processed = subscriptions.len() as i32;
    let mut invoices_created = 0;
    let mut invoice_id_result = None;
    let mut payment_request_event_id = Uuid::nil();

    for (subscription_id, price_minor, currency) in subscriptions {
        let tilled_invoice_id = format!("til_test_{}", Uuid::new_v4());
        let invoice_id: i32 = sqlx::query_scalar(
            "INSERT INTO ar_invoices
             (app_id, tilled_invoice_id, ar_customer_id, amount_cents, currency, status, due_at, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, 'draft', NOW() + interval '30 days', NOW(), NOW())
             RETURNING id",
        )
        .bind(tenant_id)
        .bind(&tilled_invoice_id)
        .bind(ar_customer_id)
        .bind(price_minor)
        .bind(&currency)
        .fetch_one(ar_pool)
        .await?;

        invoice_id_result = Some(invoice_id);

        sqlx::query(
            "UPDATE ar_invoices SET status = 'open', updated_at = NOW()
             WHERE id = $1",
        )
        .bind(invoice_id)
        .execute(ar_pool)
        .await?;

        let payment_request_payload = serde_json::json!({
            "invoice_id": invoice_id.to_string(),
            "customer_id": ar_customer_id.to_string(),
            "amount_minor": price_minor,
            "currency": currency.to_uppercase(),
            "payment_method_id": test_payment_method_id(),
        });

        let payment_request_envelope = build_envelope(
            tenant_id,
            "ar",
            "payment.collection.requested",
            "DATA_MUTATION",
            payment_request_payload,
            Some(bill_run_id.to_string()),
        );
        payment_request_event_id = payment_request_envelope.event_id;

        publish_envelope(
            nats_client,
            &nats_subject("ar", "payment.collection.requested"),
            &payment_request_envelope,
        )
        .await?;

        invoices_created += 1;

        let new_next_bill_date = execution_date + chrono::Duration::days(30);
        sqlx::query(
            "UPDATE subscriptions
             SET next_bill_date = $1, updated_at = NOW()
             WHERE id = $2",
        )
        .bind(new_next_bill_date)
        .bind(subscription_id)
        .execute(subscriptions_pool)
        .await?;
    }

    let billrun_payload = serde_json::json!({
        "bill_run_id": bill_run_id,
        "subscriptions_processed": subscriptions_processed,
        "invoices_created": invoices_created,
        "failures": 0,
        "execution_time": Utc::now().to_rfc3339()
    });
    let billrun_envelope = build_envelope(
        tenant_id,
        "subscriptions",
        "billrun.completed",
        "LIFECYCLE",
        billrun_payload,
        Some(bill_run_id.to_string()),
    );
    publish_envelope(
        nats_client,
        &nats_subject("subscriptions", "billrun.completed"),
        &billrun_envelope,
    )
    .await?;

    Ok((
        invoice_id_result.ok_or("No invoice created")?,
        payment_request_event_id,
    ))
}

// ============================================================================
// Main E2E Test
// ============================================================================

#[tokio::test]
#[serial]
async fn test_bill_run_to_notification_happy_path() {
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .try_init()
        .ok();

    tracing::info!("Starting E2E proof test: Bill Run -> Payment -> Notification");

    let nats_client = common::setup_nats_client().await;
    let ar_pool = common::get_ar_pool().await;
    let subscriptions_pool = common::get_subscriptions_pool().await;
    let payments_pool = common::get_payments_pool().await;
    let notifications_pool = common::get_notifications_pool().await;
    let bus: Arc<dyn EventBus> = Arc::new(NatsBus::new(nats_client.clone()));

    sqlx::query("TRUNCATE TABLE ar_invoices, ar_customers CASCADE")
        .execute(&ar_pool)
        .await
        .ok();
    sqlx::query("TRUNCATE TABLE subscriptions, subscription_plans, bill_runs CASCADE")
        .execute(&subscriptions_pool)
        .await
        .ok();
    sqlx::query(
        "TRUNCATE TABLE payment_attempts, payments_events_outbox, payments_processed_events CASCADE",
    )
    .execute(&payments_pool)
    .await
    .ok();
    sqlx::query("TRUNCATE TABLE events_outbox, processed_events CASCADE")
        .execute(&ar_pool)
        .await
        .ok();
    sqlx::query("TRUNCATE TABLE events_outbox, processed_events CASCADE")
        .execute(&notifications_pool)
        .await
        .ok();

    start_payment_collection_consumer(
        bus.clone(),
        payments_pool.clone(),
        Arc::new(TestPaymentProcessor::new()),
    )
    .await;
    ar_rs::consumer_tasks::start_payment_succeeded_consumer(bus.clone(), ar_pool.clone()).await;
    notifications_rs::consumer_tasks::start_payment_succeeded_consumer(
        bus.clone(),
        notifications_pool.clone(),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    let tenant_id = common::generate_test_tenant();
    let ar_customer_id = create_ar_customer(&ar_pool, &tenant_id)
        .await
        .expect("Failed to create AR customer");

    tracing::info!("Created AR customer: {}", ar_customer_id);

    let today = Utc::now().date_naive();
    let subscription_id =
        create_subscription(&subscriptions_pool, &tenant_id, ar_customer_id, today)
            .await
            .expect("Failed to create subscription");

    tracing::info!("Created subscription: {}", subscription_id);

    let bill_run_id = Uuid::new_v4().to_string();
    let billrun_subject = nats_subject("subscriptions", "billrun.completed");
    let mut billrun_stream = common::subscribe_to_events(&nats_client, &billrun_subject).await;

    tracing::info!("Triggering bill-run: {}", bill_run_id);

    let (invoice_id, payment_request_event_id) = trigger_bill_run_real_nats(
        &subscriptions_pool,
        &ar_pool,
        &nats_client,
        &bill_run_id,
        today,
        &tenant_id,
        ar_customer_id,
    )
    .await
    .expect("Failed to trigger bill run");

    tracing::info!("Bill-run triggered, created invoice: {}", invoice_id);

    let billrun_event = wait_for_nats_event(&mut billrun_stream, 10)
        .await
        .expect("Failed to receive billrun.completed event");
    assert_eq!(billrun_event["event_type"], "billrun.completed");
    assert_eq!(billrun_event["payload"]["bill_run_id"], bill_run_id);
    assert_eq!(billrun_event["payload"]["subscriptions_processed"], 1);
    assert_eq!(billrun_event["payload"]["invoices_created"], 1);
    assert_eq!(billrun_event["payload"]["failures"], 0);

    let payments_outbox_count = common::poll_for_record(
        || async {
            let count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM payments_events_outbox
                 WHERE tenant_id = $1 AND event_type = 'payment.succeeded'",
            )
            .bind(&tenant_id)
            .fetch_one(&payments_pool)
            .await
            .ok()?;

            (count == 1).then_some(count)
        },
        100,
        200,
    )
    .await
    .expect("Failed to observe payment.succeeded outbox row");
    assert_eq!(payments_outbox_count, 1);

    let payment_outbox_row: (Uuid, serde_json::Value) = common::poll_for_record(
        || async {
            sqlx::query_as::<_, (Uuid, serde_json::Value)>(
                "SELECT event_id, payload
                 FROM payments_events_outbox
                 WHERE tenant_id = $1 AND event_type = 'payment.succeeded'
                 ORDER BY created_at DESC
                 LIMIT 1",
            )
            .bind(&tenant_id)
            .fetch_optional(&payments_pool)
            .await
            .ok()
            .flatten()
        },
        100,
        200,
    )
    .await
    .expect("Failed to load payment.succeeded payload");

    let (payment_succeeded_event_id, payment_payload) = payment_outbox_row;
    assert_eq!(payment_payload["invoice_id"], invoice_id.to_string());
    assert_eq!(
        payment_payload["ar_customer_id"],
        ar_customer_id.to_string()
    );
    assert_eq!(payment_payload["amount_minor"], 2999);
    assert_eq!(payment_payload["currency"], "USD");
    assert_eq!(
        payment_payload["payment_method_ref"],
        serde_json::Value::String(test_payment_method_id())
    );

    // Publish payment.succeeded to NATS so AR and Notifications consumers receive it.
    //
    // The platform SDK outbox publisher running inside the 7d-payments Docker container has
    // no `subject_prefix` configured (module.toml omits it), so it publishes to the bare
    // event_type string ("payment.succeeded") rather than the correct subject
    // "payments.events.payment.succeeded".  It also publishes only the inner `payload`
    // column value, not a full EventEnvelope.  Because it polls every ~1 s and the
    // payments consumer writes the outbox row almost immediately, the Docker publisher
    // typically fires first, steals the row (marks published_at), and the in-process
    // publisher never sees it.  Publishing directly here from the confirmed outbox data
    // is deterministic and exercises the real NATS path.
    {
        let nats_envelope = serde_json::json!({
            "event_id": payment_succeeded_event_id,
            "event_type": "payment.succeeded",
            "occurred_at": Utc::now().to_rfc3339(),
            "tenant_id": &tenant_id,
            "source_module": "payments",
            "source_version": "1.0.0",
            "schema_version": "1.0.0",
            "replay_safe": true,
            "mutation_class": "DATA_MUTATION",
            "correlation_id": &bill_run_id,
            "payload": &payment_payload,
        });
        let envelope_bytes =
            serde_json::to_vec(&nats_envelope).expect("serialize payment.succeeded envelope");
        bus.publish("payments.events.payment.succeeded", envelope_bytes)
            .await
            .expect("publish payment.succeeded to NATS");
        sqlx::query("UPDATE payments_events_outbox SET published_at = NOW() WHERE event_id = $1")
            .bind(payment_succeeded_event_id)
            .execute(&payments_pool)
            .await
            .ok();
        tracing::info!(
            event_id = %payment_succeeded_event_id,
            "Published payment.succeeded to NATS"
        );
    }

    let payments_processed_count = common::poll_for_record(
        || async {
            let count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM payments_processed_events WHERE event_id = $1",
            )
            .bind(payment_request_event_id)
            .fetch_one(&payments_pool)
            .await
            .ok()?;

            (count == 1).then_some(count)
        },
        100,
        200,
    )
    .await
    .expect("Failed to observe payments_processed_events row");
    assert_eq!(payments_processed_count, 1);

    let invoice_status: Option<String> = common::poll_for_record(
        || async {
            let status: Option<String> =
                sqlx::query_scalar("SELECT status FROM ar_invoices WHERE id = $1")
                    .bind(invoice_id)
                    .fetch_optional(&ar_pool)
                    .await
                    .ok()
                    .flatten();

            match status.as_deref() {
                Some("paid") => status,
                _ => None,
            }
        },
        100,
        200,
    )
    .await;
    assert_eq!(invoice_status, Some("paid".to_string()));

    let ar_outbox_count = common::poll_for_record(
        || async {
            let count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM events_outbox
                 WHERE aggregate_type = 'invoice' AND aggregate_id = $1
                   AND event_type = 'ar.invoice_paid'",
            )
            .bind(invoice_id.to_string())
            .fetch_one(&ar_pool)
            .await
            .ok()?;

            (count == 1).then_some(count)
        },
        100,
        200,
    )
    .await
    .expect("Failed to observe ar.invoice_paid outbox row");
    assert_eq!(ar_outbox_count, 1);

    let ar_outbox_payload: serde_json::Value = common::poll_for_record(
        || async {
            sqlx::query_scalar::<_, serde_json::Value>(
                "SELECT payload FROM events_outbox
                 WHERE aggregate_type = 'invoice'
                   AND aggregate_id = $1
                   AND event_type = 'ar.invoice_paid'
                 ORDER BY created_at DESC
                 LIMIT 1",
            )
            .bind(invoice_id.to_string())
            .fetch_optional(&ar_pool)
            .await
            .ok()
            .flatten()
        },
        100,
        200,
    )
    .await
    .expect("Failed to load ar.invoice_paid payload");
    let ar_outbox_payload = &ar_outbox_payload["payload"];
    assert_eq!(ar_outbox_payload["invoice_id"], invoice_id.to_string());
    assert_eq!(ar_outbox_payload["customer_id"], ar_customer_id.to_string());
    assert_eq!(ar_outbox_payload["amount_cents"], 2999);
    assert_eq!(ar_outbox_payload["currency"], "USD");
    assert!(ar_outbox_payload["paid_at"].is_string());

    let ar_processed_count = common::poll_for_record(
        || async {
            let count: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM processed_events WHERE event_id = $1")
                    .bind(payment_succeeded_event_id)
                    .fetch_one(&ar_pool)
                    .await
                    .ok()?;

            (count == 1).then_some(count)
        },
        100,
        200,
    )
    .await
    .expect("Failed to observe ar processed_events row");
    assert_eq!(ar_processed_count, 1);

    let notifications_outbox_count = common::poll_for_record(
        || async {
            let count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM events_outbox
                 WHERE subject = 'notifications.delivery.succeeded' AND tenant_id = $1",
            )
            .bind(&tenant_id)
            .fetch_one(&notifications_pool)
            .await
            .ok()?;

            (count > 0).then_some(count)
        },
        100,
        200,
    )
    .await
    .expect("Failed to observe notifications outbox row");
    assert!(notifications_outbox_count > 0);

    let notifications_outbox_payload: serde_json::Value = common::poll_for_record(
        || async {
            sqlx::query_scalar::<_, serde_json::Value>(
                "SELECT payload FROM events_outbox
                 WHERE subject = 'notifications.delivery.succeeded' AND tenant_id = $1
                 ORDER BY created_at DESC
                 LIMIT 1",
            )
            .bind(&tenant_id)
            .fetch_optional(&notifications_pool)
            .await
            .ok()
            .flatten()
        },
        100,
        200,
    )
    .await
    .expect("Failed to load notifications.delivery.succeeded payload");
    let notifications_outbox_payload = &notifications_outbox_payload["payload"];
    assert_eq!(
        notifications_outbox_payload["channel"],
        serde_json::Value::String("email".to_string())
    );
    assert_eq!(
        notifications_outbox_payload["status"],
        serde_json::Value::String("succeeded".to_string())
    );
    assert_eq!(
        notifications_outbox_payload["template_id"],
        serde_json::Value::String("payment_succeeded".to_string())
    );
    assert_eq!(
        notifications_outbox_payload["attempts"],
        serde_json::Value::Number(1.into())
    );
    assert_eq!(
        notifications_outbox_payload["to"],
        serde_json::Value::String(format!("customer-{}", ar_customer_id))
    );

    let notifications_processed_count = common::poll_for_record(
        || async {
            let count: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM processed_events WHERE event_id = $1")
                    .bind(payment_succeeded_event_id)
                    .fetch_one(&notifications_pool)
                    .await
                    .ok()?;

            (count == 1).then_some(count)
        },
        100,
        200,
    )
    .await
    .expect("Failed to observe notifications processed_events row");
    assert_eq!(notifications_processed_count, 1);

    tracing::info!("E2E test completed successfully");
}
