//! Guard → Mutation → Outbox Atomicity Tests (Phase 58 Gate A, bd-mvane)
//!
//! Proves that business mutations and outbox events are written atomically.
//! After a successful mutation, the corresponding outbox row must exist.
//! After a failed guard check, no outbox row must exist.
//!
//! ## Prerequisites
//! - PostgreSQL at localhost:5443 (docker compose up -d)

use ap::domain::bills::service::create_bill;
use ap::domain::bills::{CreateBillLineRequest, CreateBillRequest};
use ap::domain::po::service::create_po;
use ap::domain::po::{CreatePoLineRequest, CreatePoRequest};
use ap::domain::vendors::qualification::change_qualification;
use ap::domain::vendors::service::create_vendor;
use ap::domain::vendors::{ChangeQualificationRequest, CreateVendorRequest, QualificationStatus};
use chrono::Utc;
use serial_test::serial;
use sqlx::{postgres::PgPoolOptions, PgPool};
use uuid::Uuid;

// ============================================================================
// Test DB helpers
// ============================================================================

async fn setup_db() -> PgPool {
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
    format!("outbox-atom-{}", Uuid::new_v4().simple())
}

fn corr() -> String {
    Uuid::new_v4().to_string()
}

async fn make_vendor(pool: &PgPool, tid: &str) -> Uuid {
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
        corr(),
    )
    .await
    .unwrap()
    .vendor_id;

    change_qualification(
        pool,
        tid,
        vendor_id,
        &ChangeQualificationRequest {
            status: QualificationStatus::Qualified,
            notes: None,
            changed_by: "test-setup".to_string(),
        },
        corr(),
    )
    .await
    .unwrap();

    vendor_id
}

// ============================================================================
// Test 1: Bill creation writes outbox event atomically
// ============================================================================

#[tokio::test]
#[serial]
async fn bill_creation_writes_outbox_atomically() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let vendor_id = make_vendor(&pool, &tid).await;

    let bill = create_bill(
        &pool,
        &tid,
        &CreateBillRequest {
            vendor_id,
            vendor_invoice_ref: "ATOM-001".to_string(),
            currency: "USD".to_string(),
            invoice_date: Utc::now(),
            due_date: None,
            tax_minor: None,
            entered_by: "atom-test".to_string(),
            fx_rate_id: None,
            lines: vec![CreateBillLineRequest {
                description: Some("Atomicity test".to_string()),
                item_id: None,
                quantity: 1.0,
                unit_price_minor: 5_000,
                gl_account_code: Some("6200".to_string()),
                po_line_id: None,
            }],
        },
        corr(),
    )
    .await
    .expect("create bill");

    // Outbox must contain the event for this bill
    let outbox_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox \
         WHERE event_type = 'ap.vendor_bill_created' \
           AND aggregate_id = $1",
    )
    .bind(bill.bill.bill_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("outbox query");

    assert_eq!(
        outbox_count, 1,
        "Exactly one outbox event for the created bill"
    );
}

// ============================================================================
// Test 2: PO creation writes outbox event atomically
// ============================================================================

#[tokio::test]
#[serial]
async fn po_creation_writes_outbox_atomically() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let vendor_id = make_vendor(&pool, &tid).await;

    let po = create_po(
        &pool,
        &tid,
        &CreatePoRequest {
            vendor_id,
            currency: "USD".to_string(),
            created_by: "atom-test".to_string(),
            expected_delivery_date: None,
            lines: vec![CreatePoLineRequest {
                item_id: None,
                description: Some("Atomicity PO line".to_string()),
                quantity: 10.0,
                unit_of_measure: "each".to_string(),
                unit_price_minor: 2_500,
                gl_account_code: "6200".to_string(),
            }],
        },
        corr(),
    )
    .await
    .expect("create PO");

    let outbox_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox \
         WHERE event_type = 'ap.po_created' \
           AND aggregate_id = $1",
    )
    .bind(po.po.po_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("outbox query");

    assert_eq!(
        outbox_count, 1,
        "Exactly one outbox event for the created PO"
    );
}

// ============================================================================
// Test 3: Failed guard check leaves no outbox residue
// ============================================================================

#[tokio::test]
#[serial]
async fn failed_guard_leaves_no_outbox_event() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let bogus_vendor_id = Uuid::new_v4(); // No such vendor

    // Count outbox rows for this tenant before the attempt.
    // Filter by payload->>'tenant_id' to isolate from concurrently-running test binaries
    // that also write bill events but with different tenant IDs.
    let before: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox \
         WHERE aggregate_type = 'bill' \
           AND payload->>'tenant_id' = $1",
    )
    .bind(&tid)
    .fetch_one(&pool)
    .await
    .expect("count before");

    // Attempt to create a bill with a non-existent vendor — guard should reject
    let result = create_bill(
        &pool,
        &tid,
        &CreateBillRequest {
            vendor_id: bogus_vendor_id,
            vendor_invoice_ref: "BOGUS-001".to_string(),
            currency: "USD".to_string(),
            invoice_date: Utc::now(),
            due_date: None,
            tax_minor: None,
            entered_by: "atom-test".to_string(),
            fx_rate_id: None,
            lines: vec![CreateBillLineRequest {
                description: Some("Should not persist".to_string()),
                item_id: None,
                quantity: 1.0,
                unit_price_minor: 1_000,
                gl_account_code: Some("6200".to_string()),
                po_line_id: None,
            }],
        },
        corr(),
    )
    .await;

    assert!(result.is_err(), "Bill creation with bogus vendor must fail");

    // No new outbox rows should have been created for this tenant
    let after: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox \
         WHERE aggregate_type = 'bill' \
           AND payload->>'tenant_id' = $1",
    )
    .bind(&tid)
    .fetch_one(&pool)
    .await
    .expect("count after");

    assert_eq!(
        before, after,
        "Failed guard check must not leave outbox residue"
    );
}
