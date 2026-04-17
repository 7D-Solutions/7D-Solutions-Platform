//! Integration test: item_id round-trips through PO create → response → event.
//!
//! Verifies bd-m8c54: item_id was previously dropped (encoded into description).
//! This test runs against the real AP database.

use ap::domain::po::service::create_po;
use ap::domain::po::{CreatePoLineRequest, CreatePoRequest};
use ap::domain::vendors::qualification::change_qualification;
use ap::domain::vendors::service::create_vendor;
use ap::domain::vendors::{ChangeQualificationRequest, CreateVendorRequest, QualificationStatus};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://ap_user:ap_pass@localhost:5443/ap_db".to_string());
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to AP test DB");
    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run AP migrations");
    pool
}

fn unique_tenant() -> String {
    format!("ap-item-rt-{}", Uuid::new_v4().simple())
}

async fn make_vendor(pool: &sqlx::PgPool, tid: &str) -> Uuid {
    let vendor_id = create_vendor(
        pool,
        tid,
        &CreateVendorRequest {
            name: format!("Vendor-{}", Uuid::new_v4().simple()),
            tax_id: None,
            currency: "USD".to_string(),
            payment_terms_days: 30,
            payment_method: Some("ach".to_string()),
            remittance_email: None,
            party_id: None,
        },
        Uuid::new_v4().to_string(),
    )
    .await
    .unwrap()
    .vendor_id;

    change_qualification(
        pool, tid, vendor_id,
        &ChangeQualificationRequest {
            status: QualificationStatus::Qualified,
            notes: None,
            changed_by: "test-setup".to_string(),
        },
        Uuid::new_v4().to_string(),
    )
    .await
    .unwrap();

    vendor_id
}

async fn cleanup(pool: &sqlx::PgPool, tid: &str) {
    let _ = sqlx::query(
        "DELETE FROM events_outbox WHERE aggregate_type = 'po' \
         AND aggregate_id IN (SELECT po_id::TEXT FROM purchase_orders WHERE tenant_id = $1)",
    )
    .bind(tid)
    .execute(pool)
    .await;
    let _ = sqlx::query(
        "DELETE FROM po_status WHERE po_id IN \
         (SELECT po_id FROM purchase_orders WHERE tenant_id = $1)",
    )
    .bind(tid)
    .execute(pool)
    .await;
    let _ = sqlx::query(
        "DELETE FROM po_lines WHERE po_id IN \
         (SELECT po_id FROM purchase_orders WHERE tenant_id = $1)",
    )
    .bind(tid)
    .execute(pool)
    .await;
    let _ = sqlx::query("DELETE FROM purchase_orders WHERE tenant_id = $1")
        .bind(tid)
        .execute(pool)
        .await;
    let _ = sqlx::query(
        "DELETE FROM events_outbox WHERE aggregate_type = 'vendor' \
         AND aggregate_id IN (SELECT vendor_id::TEXT FROM vendors WHERE tenant_id = $1)",
    )
    .bind(tid)
    .execute(pool)
    .await;
    let _ = sqlx::query("DELETE FROM vendors WHERE tenant_id = $1")
        .bind(tid)
        .execute(pool)
        .await;
}

/// item_id is persisted in po_lines and echoed in the response.
/// Before bd-m8c54 it was silently encoded into description as "item:{uuid}".
#[tokio::test]
#[serial]
async fn item_id_persists_and_is_not_in_description() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    cleanup(&pool, &tid).await;
    let vendor_id = make_vendor(&pool, &tid).await;

    let known_item_id = Uuid::new_v4();

    let req = CreatePoRequest {
        vendor_id,
        currency: "USD".to_string(),
        created_by: "user-ap".to_string(),
        expected_delivery_date: None,
        lines: vec![
            CreatePoLineRequest {
                item_id: Some(known_item_id),
                description: None,
                quantity: 3.0,
                unit_of_measure: "ea".to_string(),
                unit_price_minor: 5_000,
                gl_account_code: "6100".to_string(),
            },
            CreatePoLineRequest {
                item_id: None,
                description: Some("Misc supplies".to_string()),
                quantity: 2.0,
                unit_of_measure: "ea".to_string(),
                unit_price_minor: 1_000,
                gl_account_code: "6200".to_string(),
            },
        ],
    };

    let result = create_po(&pool, &tid, &req, Uuid::new_v4().to_string())
        .await
        .expect("create_po failed");

    // item_id round-trips
    assert_eq!(
        result.lines[0].item_id,
        Some(known_item_id),
        "item_id must be echoed in response"
    );

    // item_id must NOT be encoded into description
    assert!(
        !result.lines[0].description.starts_with("item:"),
        "item_id must not be folded into description (was the bd-m8c54 bug)"
    );

    // description-only line is unaffected
    assert!(result.lines[1].item_id.is_none());
    assert_eq!(result.lines[1].description, "Misc supplies");

    cleanup(&pool, &tid).await;
}

/// item_id appears in the ap.po_created NATS event payload.
/// Downstream (receiving, inventory projections) must be able to filter
/// receivable lines without string-parsing description.
#[tokio::test]
#[serial]
async fn item_id_appears_in_po_created_event() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    cleanup(&pool, &tid).await;
    let vendor_id = make_vendor(&pool, &tid).await;

    let known_item_id = Uuid::new_v4();

    let req = CreatePoRequest {
        vendor_id,
        currency: "USD".to_string(),
        created_by: "user-ap".to_string(),
        expected_delivery_date: None,
        lines: vec![CreatePoLineRequest {
            item_id: Some(known_item_id),
            description: None,
            quantity: 1.0,
            unit_of_measure: "ea".to_string(),
            unit_price_minor: 2_000,
            gl_account_code: "5000".to_string(),
        }],
    };

    let result = create_po(&pool, &tid, &req, Uuid::new_v4().to_string())
        .await
        .expect("create_po failed");

    let row: (serde_json::Value,) = sqlx::query_as(
        "SELECT payload FROM events_outbox WHERE aggregate_type = 'po' AND aggregate_id = $1",
    )
    .bind(result.po.po_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("outbox query failed");

    let event_item_id = row.0["payload"]["lines"][0]["item_id"]
        .as_str()
        .and_then(|s| s.parse::<Uuid>().ok());

    assert_eq!(
        event_item_id,
        Some(known_item_id),
        "ap.po_created event must carry item_id on each line"
    );

    cleanup(&pool, &tid).await;
}
