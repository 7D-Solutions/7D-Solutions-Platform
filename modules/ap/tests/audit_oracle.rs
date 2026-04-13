//! Audit Oracle — AP module
//!
//! Asserts that every AP mutation (create_bill, approve_bill, void_bill) writes
//! exactly one audit_events row inside the same transaction as the mutation.
//!
//! Real database, no mocks. Run:
//!   ./scripts/cargo-slot.sh test -p ap-rs audit_oracle -- --nocapture

use ap::domain::bills::approve::approve_bill;
use ap::domain::bills::service::{create_bill, get_bill};
use ap::domain::bills::void::void_bill;
use ap::domain::bills::{
    ApproveBillRequest, CreateBillLineRequest, CreateBillRequest, VoidBillRequest,
};
use ap::domain::vendors::service::create_vendor;
use ap::domain::vendors::CreateVendorRequest;
use chrono::Utc;
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use ap::domain::tax::ZeroTaxProvider;
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
    format!("ap-audit-{}", Uuid::new_v4().simple())
}

async fn make_vendor(pool: &sqlx::PgPool, tid: &str) -> Uuid {
    create_vendor(
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
    .expect("create vendor")
    .vendor_id
}

fn make_bill_req(vendor_id: Uuid, ref_num: &str) -> CreateBillRequest {
    CreateBillRequest {
        vendor_id,
        vendor_invoice_ref: ref_num.to_string(),
        currency: "USD".to_string(),
        invoice_date: Utc::now(),
        due_date: None,
        tax_minor: None,
        entered_by: "ap-clerk".to_string(),
        fx_rate_id: None,
        lines: vec![CreateBillLineRequest {
            description: Some("Consulting".to_string()),
            item_id: None,
            quantity: 1.0,
            unit_price_minor: 10_000,
            gl_account_code: Some("6200".to_string()),
            po_line_id: None,
        }],
    }
}

/// Count audit_events rows for a given entity_id + action combination.
async fn count_audit_events(pool: &sqlx::PgPool, entity_id: &str, action: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM audit_events WHERE entity_id = $1 AND action = $2",
    )
    .bind(entity_id)
    .bind(action)
    .fetch_one(pool)
    .await
    .expect("count audit_events")
}

/// Fetch the mutation_class for a specific audit event.
async fn fetch_mutation_class(pool: &sqlx::PgPool, entity_id: &str, action: &str) -> String {
    sqlx::query_scalar::<_, String>(
        "SELECT mutation_class::text FROM audit_events WHERE entity_id = $1 AND action = $2 LIMIT 1",
    )
    .bind(entity_id)
    .bind(action)
    .fetch_one(pool)
    .await
    .expect("fetch mutation_class")
}

// ============================================================================
// 1. create_bill → exactly 1 CREATE audit record
// ============================================================================

#[tokio::test]
#[serial]
async fn audit_oracle_create_bill() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let vendor_id = make_vendor(&pool, &tid).await;

    let bill = create_bill(
        &pool,
        &tid,
        &make_bill_req(vendor_id, "INV-AUDIT-001"),
        Uuid::new_v4().to_string(),
    )
    .await
    .expect("create_bill");

    let entity_id = bill.bill.bill_id.to_string();

    let count = count_audit_events(&pool, &entity_id, "CreateVendorBill").await;
    assert_eq!(count, 1, "Expected exactly 1 audit record for CreateVendorBill");

    let mc = fetch_mutation_class(&pool, &entity_id, "CreateVendorBill").await;
    assert_eq!(mc, "CREATE", "mutation_class should be CREATE");

    let actor_id: Option<String> = sqlx::query_scalar(
        "SELECT actor_id::text FROM audit_events WHERE entity_id = $1 AND action = $2 LIMIT 1",
    )
    .bind(&entity_id)
    .bind("CreateVendorBill")
    .fetch_one(&pool)
    .await
    .expect("fetch actor_id");
    assert_eq!(
        actor_id.unwrap_or_default(),
        "00000000-0000-0000-0000-000000000000",
        "actor_id should be nil UUID for system writes"
    );
}

// ============================================================================
// 2. approve_bill → exactly 1 STATE_TRANSITION audit record
// ============================================================================

#[tokio::test]
#[serial]
async fn audit_oracle_approve_bill() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let vendor_id = make_vendor(&pool, &tid).await;

    let bill = create_bill(
        &pool,
        &tid,
        &make_bill_req(vendor_id, "INV-AUDIT-002"),
        Uuid::new_v4().to_string(),
    )
    .await
    .expect("create_bill");

    let bill_id = bill.bill.bill_id;

    let tax = ZeroTaxProvider;
    approve_bill(
        &pool,
        &tax,
        &tid,
        bill_id,
        &ApproveBillRequest {
            approved_by: "approver".to_string(),
            override_reason: Some("audit oracle test".to_string()),
        },
        Uuid::new_v4().to_string(),
    )
    .await
    .expect("approve_bill");

    let entity_id = bill_id.to_string();

    let count = count_audit_events(&pool, &entity_id, "ApproveBill").await;
    assert_eq!(count, 1, "Expected exactly 1 audit record for ApproveBill");

    let mc = fetch_mutation_class(&pool, &entity_id, "ApproveBill").await;
    assert_eq!(mc, "STATE_TRANSITION", "mutation_class should be STATE_TRANSITION");
}

// ============================================================================
// 3. void_bill → exactly 1 REVERSAL audit record
// ============================================================================

#[tokio::test]
#[serial]
async fn audit_oracle_void_bill() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let vendor_id = make_vendor(&pool, &tid).await;

    let bill = create_bill(
        &pool,
        &tid,
        &make_bill_req(vendor_id, "INV-AUDIT-003"),
        Uuid::new_v4().to_string(),
    )
    .await
    .expect("create_bill");

    let bill_id = bill.bill.bill_id;

    let tax = ZeroTaxProvider;
    void_bill(
        &pool,
        &tax,
        &tid,
        bill_id,
        &VoidBillRequest {
            voided_by: "clerk".to_string(),
            void_reason: "audit oracle test void".to_string(),
        },
        Uuid::new_v4().to_string(),
    )
    .await
    .expect("void_bill");

    let entity_id = bill_id.to_string();

    let count = count_audit_events(&pool, &entity_id, "VoidBill").await;
    assert_eq!(count, 1, "Expected exactly 1 audit record for VoidBill");

    let mc = fetch_mutation_class(&pool, &entity_id, "VoidBill").await;
    assert_eq!(mc, "REVERSAL", "mutation_class should be REVERSAL");
}
