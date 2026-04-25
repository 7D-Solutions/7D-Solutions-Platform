//! Integration test: AR shipping_cost_consumer.
//!
//! Verifies that `handle_shipping_cost_incurred`:
//! - Appends a shipping line to a mutable invoice when customer_charge_minor is set.
//! - Emits a warning event for immutable invoices instead of silently dropping.
//! - Skips when customer_charge_minor is None or order_ref is None.
//! - Is idempotent on redelivery (processed_events table).
//!
//! Uses a real PostgreSQL database. No mocks, no stubs.

use ar_rs::consumers::shipping_cost_consumer::{
    handle_shipping_cost_incurred, ShippingCostIncurredPayload,
    EVENT_TYPE_CUSTOMER_CHARGE_AFTER_POST,
};
use chrono::Utc;
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

const TEST_TENANT: &str = "test-tenant-shipping-cost-ar";

fn db_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://ar_user:ar_pass@localhost:5444/ar_db".to_string())
}

async fn test_pool() -> PgPool {
    PgPool::connect(&db_url())
        .await
        .expect("Failed to connect to AR test DB")
}

async fn insert_customer(pool: &PgPool) -> i32 {
    let (id,): (i32,) = sqlx::query_as(
        r#"INSERT INTO ar_customers
           (app_id, external_customer_id, tilled_customer_id, status, email, name,
            payment_method_type, created_at, updated_at)
           VALUES ($1, $2, $3, 'active', 'test@example.com', 'Test Customer',
                   'card', NOW(), NOW())
           RETURNING id"#,
    )
    .bind(TEST_TENANT)
    .bind(format!("EXT-{}", Uuid::new_v4()))
    .bind(format!("TILLED-{}", Uuid::new_v4()))
    .fetch_one(pool)
    .await
    .expect("insert customer");
    id
}

async fn insert_invoice(
    pool: &PgPool,
    customer_id: i32,
    status: &str,
    correlation_id: &str,
) -> i32 {
    let tilled_invoice_id = format!("TINV-{}", Uuid::new_v4());
    let (id,): (i32,) = sqlx::query_as(
        r#"INSERT INTO ar_invoices
           (app_id, tilled_invoice_id, ar_customer_id, status, amount_cents, currency,
            correlation_id, created_at, updated_at)
           VALUES ($1, $2, $3, $4, 10000, 'USD', $5, NOW(), NOW())
           RETURNING id"#,
    )
    .bind(TEST_TENANT)
    .bind(tilled_invoice_id)
    .bind(customer_id)
    .bind(status)
    .bind(correlation_id)
    .fetch_one(pool)
    .await
    .expect("insert invoice");
    id
}

async fn cleanup(pool: &PgPool) {
    sqlx::query(
        "DELETE FROM processed_events WHERE processor = 'ar-shipping-cost-consumer' \
         AND event_type = 'shipping_receiving.shipping_cost.incurred'",
    )
    .execute(pool)
    .await
    .ok();
    sqlx::query(
        "DELETE FROM events_outbox WHERE event_type = $1",
    )
    .bind(EVENT_TYPE_CUSTOMER_CHARGE_AFTER_POST)
    .execute(pool)
    .await
    .ok();
    sqlx::query(
        "DELETE FROM ar_invoices WHERE app_id = $1",
    )
    .bind(TEST_TENANT)
    .execute(pool)
    .await
    .ok();
    sqlx::query("DELETE FROM ar_customers WHERE app_id = $1")
        .bind(TEST_TENANT)
        .execute(pool)
        .await
        .ok();
}

fn sample_payload(order_ref: Option<&str>, customer_charge: Option<i64>) -> ShippingCostIncurredPayload {
    ShippingCostIncurredPayload {
        tenant_id: TEST_TENANT.to_string(),
        shipment_id: Uuid::new_v4(),
        tracking_number: format!("TRACK-{}", Uuid::new_v4()),
        carrier_code: "ups".to_string(),
        customer_charge_minor: customer_charge,
        currency: "USD".to_string(),
        order_ref: order_ref.map(|s| s.to_string()),
        incurred_at: Utc::now(),
    }
}

#[tokio::test]
#[serial]
async fn test_adds_shipping_line_to_open_invoice() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    let customer_id = insert_customer(&pool).await;
    let order_ref = format!("ORDER-{}", Uuid::new_v4());
    let invoice_id = insert_invoice(&pool, customer_id, "open", &order_ref).await;

    let payload = sample_payload(Some(&order_ref), Some(1500));
    let event_id = Uuid::new_v4();

    handle_shipping_cost_incurred(&pool, event_id, &payload)
        .await
        .expect("handler failed");

    let (amount_cents, line_item_details): (i64, Option<serde_json::Value>) = sqlx::query_as(
        "SELECT amount_cents, line_item_details FROM ar_invoices WHERE id = $1",
    )
    .bind(invoice_id)
    .fetch_one(&pool)
    .await
    .expect("fetch invoice");

    assert_eq!(amount_cents, 11500, "amount_cents should include shipping charge");
    let lines = line_item_details.expect("line_item_details should not be null");
    let arr = lines.as_array().expect("line_item_details should be an array");
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["amount_cents"], 1500);
    assert!(arr[0]["description"].as_str().unwrap().contains("ups"));

    cleanup(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_emits_warning_for_paid_invoice() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    let customer_id = insert_customer(&pool).await;
    let order_ref = format!("ORDER-PAID-{}", Uuid::new_v4());
    let invoice_id = insert_invoice(&pool, customer_id, "paid", &order_ref).await;

    let payload = sample_payload(Some(&order_ref), Some(800));

    handle_shipping_cost_incurred(&pool, Uuid::new_v4(), &payload)
        .await
        .expect("handler failed");

    // Invoice amount should NOT have changed.
    let (amount_cents,): (i64,) =
        sqlx::query_as("SELECT amount_cents FROM ar_invoices WHERE id = $1")
            .bind(invoice_id)
            .fetch_one(&pool)
            .await
            .expect("fetch invoice");
    assert_eq!(amount_cents, 10000, "paid invoice amount must not change");

    // Warning event should be in the outbox.
    let (count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM events_outbox WHERE event_type = $1 AND aggregate_id = $2",
    )
    .bind(EVENT_TYPE_CUSTOMER_CHARGE_AFTER_POST)
    .bind(invoice_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("count warning events");
    assert_eq!(count, 1, "one warning event should be emitted");

    cleanup(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_skips_when_no_customer_charge() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    let customer_id = insert_customer(&pool).await;
    let order_ref = format!("ORDER-FREE-{}", Uuid::new_v4());
    let invoice_id = insert_invoice(&pool, customer_id, "open", &order_ref).await;

    let payload = sample_payload(Some(&order_ref), None); // no customer charge

    handle_shipping_cost_incurred(&pool, Uuid::new_v4(), &payload)
        .await
        .expect("handler failed");

    let (amount_cents,): (i64,) =
        sqlx::query_as("SELECT amount_cents FROM ar_invoices WHERE id = $1")
            .bind(invoice_id)
            .fetch_one(&pool)
            .await
            .expect("fetch invoice");
    assert_eq!(amount_cents, 10000, "invoice total should not change when customer charge is None");

    cleanup(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_idempotent_on_redelivery() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    let customer_id = insert_customer(&pool).await;
    let order_ref = format!("ORDER-IDEM-{}", Uuid::new_v4());
    insert_invoice(&pool, customer_id, "open", &order_ref).await;

    let payload = sample_payload(Some(&order_ref), Some(500));
    let event_id = Uuid::new_v4();

    handle_shipping_cost_incurred(&pool, event_id, &payload)
        .await
        .expect("first call failed");
    handle_shipping_cost_incurred(&pool, event_id, &payload)
        .await
        .expect("second call must not error");

    // Invoice should only have been updated once.
    let (amount_cents,): (i64,) = sqlx::query_as(
        "SELECT amount_cents FROM ar_invoices WHERE app_id = $1 AND correlation_id = $2",
    )
    .bind(TEST_TENANT)
    .bind(&order_ref)
    .fetch_one(&pool)
    .await
    .expect("fetch invoice");
    assert_eq!(amount_cents, 10500, "idempotent: amount should only increase once");

    cleanup(&pool).await;
}
