/// Integration tests for bd-mozdb: shipping notification templates + consumers.
///
/// All tests run against real Postgres — no mocks, no stubs.
/// Template rendering tests run in-memory (no I/O).
use chrono::Utc;
use event_bus::BusMessage;
use notifications_rs::{
    consumers::{
        shipping::{
            handle_outbound_delivered, handle_outbound_shipped, OutboundDeliveredPayload,
            OutboundShippedLine, OutboundShippedPayload,
        },
        EventConsumer,
    },
    models::EnvelopeMetadata,
    templates::render,
};
use serial_test::serial;
use sqlx::PgPool;
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

fn unique_tenant() -> String {
    Uuid::new_v4().to_string()
}

// ── Migration seed verification ───────────────────────────────────────────────

/// Seed templates must be present in the DB after migration runs.
#[tokio::test]
#[serial]
async fn shipping_seed_templates_exist_in_db() {
    let pool = get_pool().await;

    let (order_shipped_count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM notification_templates WHERE template_key = 'order_shipped'",
    )
    .fetch_one(&pool)
    .await
    .expect("query order_shipped template");

    assert!(
        order_shipped_count >= 1,
        "order_shipped seed template must exist after migration"
    );

    let (delivery_confirmed_count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM notification_templates WHERE template_key = 'delivery_confirmed'",
    )
    .fetch_one(&pool)
    .await
    .expect("query delivery_confirmed template");

    assert!(
        delivery_confirmed_count >= 1,
        "delivery_confirmed seed template must exist after migration"
    );
}

// ── Shipped consumer ─────────────────────────────────────────────────────────

/// Outbound shipped event creates a notification_sends row with the correct
/// template_key and all required template variables.
#[tokio::test]
#[serial]
async fn shipping_outbound_shipped_creates_send_request() {
    let pool = get_pool().await;
    let tenant = unique_tenant();
    let shipment_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    let so_id = Uuid::new_v4();

    let payload = OutboundShippedPayload {
        tenant_id: tenant.clone(),
        shipment_id,
        lines: vec![OutboundShippedLine {
            line_id: Uuid::new_v4(),
            sku: "SKU-001".to_string(),
            qty_shipped: 2,
            issue_id: None,
            source_ref_type: Some("sales_order".to_string()),
            source_ref_id: Some(so_id),
        }],
        shipped_at: Utc::now(),
        tracking_number: Some("1Z999AA10123456784".to_string()),
        carrier_party_id: None,
    };

    let metadata = EnvelopeMetadata {
        event_id,
        tenant_id: tenant.clone(),
        correlation_id: None,
    };

    handle_outbound_shipped(&pool, payload, metadata)
        .await
        .expect("handle_outbound_shipped failed");

    // Verify notification_sends row exists with correct template_key
    let (template_key, channel, payload_json): (Option<String>, String, serde_json::Value) =
        sqlx::query_as(
            "SELECT template_key, channel, payload_json \
             FROM notification_sends \
             WHERE tenant_id = $1 AND causation_id = $2 \
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(&tenant)
        .bind(event_id.to_string())
        .fetch_one(&pool)
        .await
        .expect("notification_sends row not found after handle_outbound_shipped");

    assert_eq!(template_key.as_deref(), Some("order_shipped"));
    assert_eq!(channel, "email");

    // Verify all required template vars are present and correct
    assert_eq!(
        payload_json["tracking_number"].as_str(),
        Some("1Z999AA10123456784"),
        "tracking_number should be in payload_json"
    );
    assert_eq!(
        payload_json["carrier"].as_str(),
        Some("unknown"),
        "carrier should default to unknown when carrier_party_id is None"
    );
    assert!(
        payload_json["shipped_at"].as_str().is_some(),
        "shipped_at must be in payload_json"
    );
    assert_eq!(
        payload_json["recipient_name"].as_str(),
        Some("Customer"),
        "recipient_name must be in payload_json"
    );
}

/// Delivered event creates a notification_sends row with the correct template_key.
#[tokio::test]
#[serial]
async fn shipping_outbound_delivered_creates_send_request() {
    let pool = get_pool().await;
    let tenant = unique_tenant();
    let shipment_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();

    let payload = OutboundDeliveredPayload {
        tenant_id: tenant.clone(),
        shipment_id,
        delivered_at: Utc::now(),
    };

    let metadata = EnvelopeMetadata {
        event_id,
        tenant_id: tenant.clone(),
        correlation_id: None,
    };

    handle_outbound_delivered(&pool, payload, metadata)
        .await
        .expect("handle_outbound_delivered failed");

    let (template_key, channel, payload_json): (Option<String>, String, serde_json::Value) =
        sqlx::query_as(
            "SELECT template_key, channel, payload_json \
             FROM notification_sends \
             WHERE tenant_id = $1 AND causation_id = $2 \
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(&tenant)
        .bind(event_id.to_string())
        .fetch_one(&pool)
        .await
        .expect("notification_sends row not found after handle_outbound_delivered");

    assert_eq!(template_key.as_deref(), Some("delivery_confirmed"));
    assert_eq!(channel, "email");

    assert!(
        payload_json["delivered_at"].as_str().is_some(),
        "delivered_at must be in payload_json"
    );
    assert_eq!(
        payload_json["recipient_name"].as_str(),
        Some("Customer"),
        "recipient_name must be in payload_json"
    );
}

// ── Idempotency ───────────────────────────────────────────────────────────────

/// Re-publishing the same event (same event_id) must not create a duplicate
/// notification_sends row — the processed_events gate short-circuits.
#[tokio::test]
#[serial]
async fn shipping_idempotency_no_duplicate_send() {
    let pool = get_pool().await;
    let tenant = unique_tenant();
    let shipment_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();

    // Build a minimal but valid event envelope for process_idempotent.
    let envelope_bytes = serde_json::to_vec(&serde_json::json!({
        "event_id": event_id.to_string(),
        "tenant_id": &tenant,
        "source_module": "shipping-receiving",
        "source_version": "1.0.0",
        "occurred_at": Utc::now().to_rfc3339(),
        "payload": {
            "tenant_id": &tenant,
            "shipment_id": shipment_id.to_string(),
            "lines": [],
            "shipped_at": Utc::now().to_rfc3339(),
            "tracking_number": null,
            "carrier_party_id": null,
        }
    }))
    .unwrap();

    let msg = BusMessage::new(
        "shipping_receiving.outbound_shipped".to_string(),
        envelope_bytes,
    );

    let consumer = EventConsumer::new(pool.clone());

    // First call — handler runs, inserts 1 notification_sends row.
    {
        let pool2 = pool.clone();
        let tenant2 = tenant.clone();
        consumer
            .process_idempotent::<OutboundShippedPayload, _, _>(
                &msg,
                move |p: OutboundShippedPayload| {
                    let pool2 = pool2.clone();
                    let tenant2 = tenant2.clone();
                    async move {
                        let m = EnvelopeMetadata {
                            event_id,
                            tenant_id: tenant2,
                            correlation_id: None,
                        };
                        handle_outbound_shipped(&pool2, p, m).await
                    }
                },
            )
            .await
            .expect("first process_idempotent failed");
    }

    // Second call — idempotency gate fires, handler is never invoked.
    {
        let pool2 = pool.clone();
        let tenant2 = tenant.clone();
        consumer
            .process_idempotent::<OutboundShippedPayload, _, _>(
                &msg,
                move |p: OutboundShippedPayload| {
                    let pool2 = pool2.clone();
                    let tenant2 = tenant2.clone();
                    async move {
                        let m = EnvelopeMetadata {
                            event_id,
                            tenant_id: tenant2,
                            correlation_id: None,
                        };
                        handle_outbound_shipped(&pool2, p, m).await
                    }
                },
            )
            .await
            .expect("second process_idempotent failed");
    }

    // Exactly 1 notification_sends row for this event.
    let (count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM notification_sends WHERE tenant_id = $1 AND causation_id = $2",
    )
    .bind(&tenant)
    .bind(event_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("count query failed");

    assert_eq!(
        count, 1,
        "exactly 1 notification_sends row expected — idempotency gate should block duplicate"
    );
}

// ── Missing tracking_number defaults ────────────────────────────────────────

/// When tracking_number is None, the template variable must be "pending".
#[tokio::test]
#[serial]
async fn shipping_missing_tracking_number_defaults_to_pending() {
    let pool = get_pool().await;
    let tenant = unique_tenant();
    let shipment_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();

    let payload = OutboundShippedPayload {
        tenant_id: tenant.clone(),
        shipment_id,
        lines: vec![],
        shipped_at: Utc::now(),
        tracking_number: None,
        carrier_party_id: None,
    };

    let metadata = EnvelopeMetadata {
        event_id,
        tenant_id: tenant.clone(),
        correlation_id: None,
    };

    handle_outbound_shipped(&pool, payload, metadata)
        .await
        .expect("handle_outbound_shipped failed");

    let (payload_json,): (serde_json::Value,) = sqlx::query_as(
        "SELECT payload_json FROM notification_sends \
         WHERE tenant_id = $1 AND causation_id = $2 \
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(&tenant)
    .bind(event_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("notification_sends row not found");

    assert_eq!(
        payload_json["tracking_number"].as_str(),
        Some("pending"),
        "tracking_number = None must default to 'pending'"
    );
    assert_eq!(
        payload_json["carrier"].as_str(),
        Some("unknown"),
        "carrier_party_id = None must default to 'unknown'"
    );
}

// ── Template rendering ────────────────────────────────────────────────────────

/// order_shipped template renders correctly — subject contains tracking number,
/// body is valid HTML.
#[test]
fn shipping_template_order_shipped_renders_correctly() {
    let payload = serde_json::json!({
        "tracking_number": "1Z999AA10123456784",
        "carrier": "UPS",
        "shipped_at": "2026-04-08T12:00:00Z",
        "recipient_name": "Alice",
    });

    let msg = render("order_shipped", &payload).expect("render order_shipped failed");

    assert!(
        msg.subject.contains("1Z999AA10123456784"),
        "subject must contain tracking number, got: {}",
        msg.subject
    );
    assert!(
        msg.body_html.contains("<p>"),
        "body_html must be HTML, got: {}",
        msg.body_html
    );
    assert!(
        msg.body_html.contains("UPS"),
        "body_html must contain carrier, got: {}",
        msg.body_html
    );
    assert!(
        msg.body_html.contains("1Z999AA10123456784"),
        "body_html must contain tracking number"
    );
    assert!(
        msg.body_text.contains("Alice"),
        "body_text must contain recipient_name"
    );
}

/// delivery_confirmed template renders correctly.
#[test]
fn shipping_template_delivery_confirmed_renders_correctly() {
    let payload = serde_json::json!({
        "delivered_at": "2026-04-10T14:30:00Z",
        "recipient_name": "Bob",
    });

    let msg = render("delivery_confirmed", &payload).expect("render delivery_confirmed failed");

    assert_eq!(msg.subject, "Your order has been delivered");
    assert!(
        msg.body_html.contains("<p>"),
        "body_html must be HTML"
    );
    assert!(
        msg.body_html.contains("delivered"),
        "body_html must mention delivery"
    );
    assert!(
        msg.body_text.contains("Bob"),
        "body_text must contain recipient_name"
    );
    assert!(
        msg.body_text.contains("2026-04-10"),
        "body_text must contain delivered_at"
    );
}

/// order_shipped template with "pending" tracking number renders without error.
#[test]
fn shipping_template_order_shipped_pending_tracking() {
    let payload = serde_json::json!({
        "tracking_number": "pending",
        "carrier": "unknown",
        "shipped_at": "2026-04-08T10:00:00Z",
        "recipient_name": "Customer",
    });

    let msg = render("order_shipped", &payload).expect("render with pending tracking should work");
    assert!(msg.subject.contains("pending"));
}
