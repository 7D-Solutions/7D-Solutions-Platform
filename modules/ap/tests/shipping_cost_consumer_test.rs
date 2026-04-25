//! Integration test: AP shipping_cost_consumer.
//!
//! Verifies that `handle_shipping_cost_incurred` creates a vendor_bill and
//! bill_line for a configured carrier, and is idempotent on redelivery.
//! Uses a real PostgreSQL database. No mocks, no stubs.

use ap::consumers::shipping_cost_consumer::{
    handle_shipping_cost_incurred, ShippingCostIncurredPayload,
};
use chrono::Utc;
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

const TEST_TENANT: &str = "test-tenant-shipping-cost-ap";

fn db_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://ap_user:ap_pass@localhost:5443/ap_db".to_string())
}

async fn test_pool() -> PgPool {
    PgPool::connect(&db_url())
        .await
        .expect("Failed to connect to AP test DB")
}

async fn setup_vendor(pool: &PgPool) -> Uuid {
    let vendor_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO vendors (vendor_id, tenant_id, name, currency, payment_terms_days,
           is_active, created_at, updated_at)
           VALUES ($1, $2, $3, 'USD', 30, TRUE, NOW(), NOW())"#,
    )
    .bind(vendor_id)
    .bind(TEST_TENANT)
    .bind(format!("UPS-Carrier-{}", vendor_id))
    .execute(pool)
    .await
    .expect("insert vendor");
    vendor_id
}

async fn setup_carrier_mapping(pool: &PgPool, vendor_id: Uuid, carrier_code: &str) {
    sqlx::query(
        r#"INSERT INTO ap_carrier_vendor_mapping (tenant_id, carrier_code, vendor_id, default_gl_account_code)
           VALUES ($1, $2, $3, '6200')
           ON CONFLICT (tenant_id, carrier_code) DO NOTHING"#,
    )
    .bind(TEST_TENANT)
    .bind(carrier_code)
    .bind(vendor_id)
    .execute(pool)
    .await
    .expect("insert carrier mapping");
}

async fn cleanup(pool: &PgPool) {
    sqlx::query(
        "DELETE FROM bill_lines WHERE bill_id IN \
         (SELECT bill_id FROM vendor_bills WHERE tenant_id = $1)",
    )
    .bind(TEST_TENANT)
    .execute(pool)
    .await
    .ok();
    sqlx::query("DELETE FROM vendor_bills WHERE tenant_id = $1")
        .bind(TEST_TENANT)
        .execute(pool)
        .await
        .ok();
    sqlx::query(
        "DELETE FROM ap_carrier_vendor_mapping WHERE tenant_id = $1",
    )
    .bind(TEST_TENANT)
    .execute(pool)
    .await
    .ok();
    sqlx::query(
        "DELETE FROM events_outbox WHERE aggregate_type = 'vendor' \
         AND aggregate_id IN (SELECT vendor_id::TEXT FROM vendors WHERE tenant_id = $1)",
    )
    .bind(TEST_TENANT)
    .execute(pool)
    .await
    .ok();
    sqlx::query("DELETE FROM vendors WHERE tenant_id = $1")
        .bind(TEST_TENANT)
        .execute(pool)
        .await
        .ok();
}

fn sample_payload(tracking: &str) -> ShippingCostIncurredPayload {
    ShippingCostIncurredPayload {
        tenant_id: TEST_TENANT.to_string(),
        shipment_id: Uuid::new_v4(),
        tracking_number: tracking.to_string(),
        carrier_code: "ups".to_string(),
        charge_minor: 1500,
        currency: "USD".to_string(),
        incurred_at: Utc::now(),
    }
}

#[tokio::test]
#[serial]
async fn test_creates_bill_and_line_for_configured_carrier() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    let vendor_id = setup_vendor(&pool).await;
    setup_carrier_mapping(&pool, vendor_id, "ups").await;

    let payload = sample_payload("1Z999AA10123456784");
    let event_id = Uuid::new_v4();

    handle_shipping_cost_incurred(&pool, event_id, &payload)
        .await
        .expect("handler failed");

    let (bill_count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM vendor_bills WHERE tenant_id = $1 AND vendor_id = $2 AND vendor_invoice_ref = $3",
    )
    .bind(TEST_TENANT)
    .bind(vendor_id)
    .bind(&payload.tracking_number)
    .fetch_one(&pool)
    .await
    .expect("count bills");
    assert_eq!(bill_count, 1, "one bill should be created");

    let (line_count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM bill_lines WHERE bill_id IN \
         (SELECT bill_id FROM vendor_bills WHERE tenant_id = $1 AND vendor_invoice_ref = $2)",
    )
    .bind(TEST_TENANT)
    .bind(&payload.tracking_number)
    .fetch_one(&pool)
    .await
    .expect("count lines");
    assert_eq!(line_count, 1, "one bill line should be created");

    cleanup(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_idempotent_on_duplicate_event() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    let vendor_id = setup_vendor(&pool).await;
    setup_carrier_mapping(&pool, vendor_id, "ups").await;

    let payload = sample_payload("IDEM-TRACK-001");
    let event_id = Uuid::new_v4();

    handle_shipping_cost_incurred(&pool, event_id, &payload)
        .await
        .expect("first call failed");
    handle_shipping_cost_incurred(&pool, Uuid::new_v4(), &payload)
        .await
        .expect("second call must not error");

    let (count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM vendor_bills WHERE tenant_id = $1 AND vendor_invoice_ref = $2",
    )
    .bind(TEST_TENANT)
    .bind(&payload.tracking_number)
    .fetch_one(&pool)
    .await
    .expect("count");
    assert_eq!(count, 1, "idempotent: duplicate event must not create a second bill");

    cleanup(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_skips_when_carrier_not_configured() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    // No mapping for "fedex" — handler should skip gracefully.
    let mut payload = sample_payload("7489083400301487591");
    payload.carrier_code = "fedex".to_string();

    handle_shipping_cost_incurred(&pool, Uuid::new_v4(), &payload)
        .await
        .expect("handler must not error on missing mapping");

    let (count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM vendor_bills WHERE tenant_id = $1 AND vendor_invoice_ref = $2",
    )
    .bind(TEST_TENANT)
    .bind(&payload.tracking_number)
    .fetch_one(&pool)
    .await
    .expect("count");
    assert_eq!(count, 0, "no bill created when carrier is not configured");

    cleanup(&pool).await;
}

#[tokio::test]
#[serial]
async fn test_carrier_mapping_lookup_correct_vendor() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    let vendor_id = setup_vendor(&pool).await;
    setup_carrier_mapping(&pool, vendor_id, "ups").await;

    let payload = sample_payload("VENDOR-MATCH-001");
    handle_shipping_cost_incurred(&pool, Uuid::new_v4(), &payload)
        .await
        .expect("handler failed");

    let (actual_vendor_id,): (Uuid,) = sqlx::query_as(
        "SELECT vendor_id FROM vendor_bills WHERE tenant_id = $1 AND vendor_invoice_ref = $2",
    )
    .bind(TEST_TENANT)
    .bind(&payload.tracking_number)
    .fetch_one(&pool)
    .await
    .expect("fetch bill");
    assert_eq!(actual_vendor_id, vendor_id, "bill must be assigned to the mapped vendor");

    cleanup(&pool).await;
}
