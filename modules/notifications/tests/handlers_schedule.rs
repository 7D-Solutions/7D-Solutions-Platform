/// DB integration tests for notification scheduling producers.
///
/// Validates that `handle_invoice_issued()` and `handle_payment_failed()`
/// insert the correct rows into `scheduled_notifications`.  No dispatcher
/// is involved — this is purely producer-side verification.
use chrono::{Duration, Utc};
use notifications_rs::{
    handlers::{handle_invoice_issued, handle_payment_failed},
    models::{EnvelopeMetadata, InvoiceIssuedPayload, PaymentFailedPayload},
};
use sqlx::PgPool;
use uuid::Uuid;

const DEFAULT_DB_URL: &str =
    "postgresql://notifications_user:notifications_pass@localhost:5437/notifications_db";

/// Returns a connected pool and ensures all migrations are applied.
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

/// Minimal row shape used only within these tests.
#[derive(sqlx::FromRow)]
struct ScheduledRow {
    template_key: String,
    channel: String,
    deliver_at: chrono::DateTime<Utc>,
    recipient_ref: String,
    payload_json: serde_json::Value,
}

// ─────────────────────────────────────────────────────────────────────────────
// invoice_due_soon
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_invoice_issued_inserts_due_soon_row() {
    let pool = get_pool().await;

    let tenant_id = Uuid::new_v4().to_string();
    let customer_id = Uuid::new_v4().to_string();
    let invoice_id = Uuid::new_v4().to_string();

    // due 30 days from now
    let due_date_dt = Utc::now() + Duration::days(30);
    let due_date = due_date_dt.format("%Y-%m-%d").to_string();

    let payload = InvoiceIssuedPayload {
        invoice_id: invoice_id.clone(),
        customer_id: customer_id.clone(),
        amount_due_minor: 5000,
        currency: "USD".to_string(),
        due_date: Some(due_date.clone()),
    };
    let metadata = EnvelopeMetadata {
        event_id: Uuid::new_v4(),
        tenant_id: tenant_id.clone(),
        correlation_id: None,
    };

    handle_invoice_issued(&pool, payload, metadata)
        .await
        .expect("handler returned an error");

    let recipient_ref = format!("{}:{}", tenant_id, customer_id);
    let row: ScheduledRow = sqlx::query_as(
        r#"
        SELECT template_key, channel, deliver_at, recipient_ref, payload_json
        FROM scheduled_notifications
        WHERE recipient_ref = $1 AND template_key = 'invoice_due_soon'
        ORDER BY created_at DESC
        LIMIT 1
        "#,
    )
    .bind(&recipient_ref)
    .fetch_one(&pool)
    .await
    .expect("No scheduled_notifications row found after handle_invoice_issued");

    assert_eq!(row.template_key, "invoice_due_soon");
    assert_eq!(row.channel, "email");
    assert_eq!(row.recipient_ref, recipient_ref);

    // deliver_at = due_date midnight UTC - 3 days
    let expected = chrono::NaiveDate::parse_from_str(&due_date, "%Y-%m-%d")
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
        - Duration::days(3);
    let delta = (row.deliver_at - expected).num_seconds().abs();
    assert!(
        delta < 5,
        "deliver_at {} differs from expected {} by {}s",
        row.deliver_at,
        expected,
        delta
    );

    // payload contains invoice_id and due_date
    let pj = &row.payload_json;
    assert_eq!(
        pj["invoice_id"].as_str(),
        Some(invoice_id.as_str()),
        "payload_json.invoice_id mismatch"
    );
    assert_eq!(
        pj["due_date"].as_str(),
        Some(due_date.as_str()),
        "payload_json.due_date mismatch"
    );
}

#[tokio::test]
async fn test_invoice_issued_no_due_date_skips_insert() {
    let pool = get_pool().await;

    let tenant_id = Uuid::new_v4().to_string();
    let customer_id = Uuid::new_v4().to_string();
    let invoice_id = Uuid::new_v4().to_string();

    let payload = InvoiceIssuedPayload {
        invoice_id: invoice_id.clone(),
        customer_id: customer_id.clone(),
        amount_due_minor: 1000,
        currency: "USD".to_string(),
        due_date: None,
    };
    let metadata = EnvelopeMetadata {
        event_id: Uuid::new_v4(),
        tenant_id: tenant_id.clone(),
        correlation_id: None,
    };

    handle_invoice_issued(&pool, payload, metadata)
        .await
        .expect("handler returned an error");

    let recipient_ref = format!("{}:{}", tenant_id, customer_id);
    let count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM scheduled_notifications WHERE recipient_ref = $1")
            .bind(&recipient_ref)
            .fetch_one(&pool)
            .await
            .expect("count query failed");

    assert_eq!(
        count.0, 0,
        "Expected no rows when due_date is absent, got {}",
        count.0
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// payment_retry
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_payment_failed_inserts_retry_row() {
    let pool = get_pool().await;

    let tenant_id = Uuid::new_v4().to_string();
    let customer_id = Uuid::new_v4().to_string();
    let payment_id = Uuid::new_v4().to_string();
    let invoice_id = Uuid::new_v4().to_string();

    let before = Utc::now();

    let payload = PaymentFailedPayload {
        payment_id: payment_id.clone(),
        invoice_id: invoice_id.clone(),
        ar_customer_id: customer_id.clone(),
        amount_minor: 10000,
        currency: "USD".to_string(),
        failure_code: "insufficient_funds".to_string(),
        failure_message: None,
        processor_payment_id: None,
        attempts: None,
    };
    let metadata = EnvelopeMetadata {
        event_id: Uuid::new_v4(),
        tenant_id: tenant_id.clone(),
        correlation_id: None,
    };

    handle_payment_failed(&pool, payload, metadata)
        .await
        .expect("handler returned an error");

    let after = Utc::now();

    let recipient_ref = format!("{}:{}", tenant_id, customer_id);
    let row: ScheduledRow = sqlx::query_as(
        r#"
        SELECT template_key, channel, deliver_at, recipient_ref, payload_json
        FROM scheduled_notifications
        WHERE recipient_ref = $1 AND template_key = 'payment_retry'
        ORDER BY created_at DESC
        LIMIT 1
        "#,
    )
    .bind(&recipient_ref)
    .fetch_one(&pool)
    .await
    .expect("No scheduled_notifications row found after handle_payment_failed");

    assert_eq!(row.template_key, "payment_retry");
    assert_eq!(row.channel, "email");
    assert_eq!(row.recipient_ref, recipient_ref);

    // deliver_at = now + 24h — verify window [before+24h, after+24h+5s]
    let expected_min = before + Duration::hours(24);
    let expected_max = after + Duration::hours(24) + Duration::seconds(5);
    assert!(
        row.deliver_at >= expected_min && row.deliver_at <= expected_max,
        "deliver_at {} not in expected window [{}, {}]",
        row.deliver_at,
        expected_min,
        expected_max
    );

    // payload contains payment_id, invoice_id, failure_code
    let pj = &row.payload_json;
    assert_eq!(
        pj["payment_id"].as_str(),
        Some(payment_id.as_str()),
        "payload_json.payment_id mismatch"
    );
    assert_eq!(
        pj["invoice_id"].as_str(),
        Some(invoice_id.as_str()),
        "payload_json.invoice_id mismatch"
    );
    assert_eq!(
        pj["failure_code"].as_str(),
        Some("insufficient_funds"),
        "payload_json.failure_code mismatch"
    );
}
