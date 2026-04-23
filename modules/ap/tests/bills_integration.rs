//! Integration tests for AP bill lifecycle: create/match/approve/void/tax-quote (bd-3rvu).
//!
//! Covers:
//! 1. Create bill — happy path
//! 2. Duplicate invoice ref rejection
//! 3. Match engine — PO not found error case
//! 4. Match engine — two_way match with real PO
//! 5. Approve open bill (with override_reason)
//! 6. Void open bill
//! 7. Void already-voided bill → InvalidTransition
//! 8. Quote tax on bill (ZeroTaxProvider)
//! 9. Tenant isolation

use ap::domain::bills::approve::approve_bill;
use ap::domain::bills::service::{create_bill, get_bill};
use ap::domain::bills::void::void_bill;
use ap::domain::bills::{
    ApproveBillRequest, BillError, CreateBillLineRequest, CreateBillRequest, VoidBillRequest,
};
use ap::domain::po::service::create_po;
use ap::domain::po::{CreatePoLineRequest, CreatePoRequest};
use ap::domain::r#match::service::run_match;
use ap::domain::r#match::{MatchError, RunMatchRequest};
use ap::domain::tax::{quote_bill_tax, TaxAddress, ZeroTaxProvider};
use ap::domain::vendors::qualification::change_qualification;
use ap::domain::vendors::service::create_vendor;
use ap::domain::vendors::{ChangeQualificationRequest, CreateVendorRequest, QualificationStatus};
use chrono::Utc;
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use tax_core::models::{TaxLineItem, TaxQuoteRequest};
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
    format!("ap-bill-{}", Uuid::new_v4().simple())
}

fn corr() -> String {
    Uuid::new_v4().to_string()
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
        pool,
        tid,
        vendor_id,
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
            description: Some("Consulting services".to_string()),
            item_id: None,
            quantity: 5.0,
            unit_price_minor: 10_000,
            gl_account_code: Some("6200".to_string()),
            po_line_id: None,
        }],
    }
}

// ============================================================================
// 1. Create bill — happy path
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_bill() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let vendor_id = make_vendor(&pool, &tid).await;

    let bill = create_bill(&pool, &tid, &make_bill_req(vendor_id, "INV-001"), corr())
        .await
        .unwrap();

    assert_eq!(bill.bill.status, "open");
    assert_eq!(bill.bill.total_minor, 50_000); // 5 × 10_000
    assert_eq!(bill.lines.len(), 1);

    let fetched = get_bill(&pool, &tid, bill.bill.bill_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fetched.bill.bill_id, bill.bill.bill_id);
}

// ============================================================================
// 2. Duplicate invoice ref → DuplicateInvoice error
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_bill_duplicate_invoice_rejected() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let vendor_id = make_vendor(&pool, &tid).await;

    create_bill(&pool, &tid, &make_bill_req(vendor_id, "INV-DUP"), corr())
        .await
        .unwrap();

    let err = create_bill(&pool, &tid, &make_bill_req(vendor_id, "INV-DUP"), corr())
        .await
        .unwrap_err();

    assert!(
        matches!(err, BillError::DuplicateInvoice(_)),
        "expected DuplicateInvoice, got: {:?}",
        err
    );
}

// ============================================================================
// 3. Match engine — PoNotFound when PO does not exist
// ============================================================================

#[tokio::test]
#[serial]
async fn test_run_match_po_not_found() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let vendor_id = make_vendor(&pool, &tid).await;

    let bill = create_bill(
        &pool,
        &tid,
        &make_bill_req(vendor_id, "INV-NOMATCH"),
        corr(),
    )
    .await
    .unwrap();

    let err = run_match(
        &pool,
        &tid,
        bill.bill.bill_id,
        &RunMatchRequest {
            po_id: Uuid::new_v4(), // nonexistent
            matched_by: "matcher-1".to_string(),
            price_tolerance_pct: 0.05,
        },
        corr(),
    )
    .await
    .unwrap_err();

    assert!(
        matches!(err, MatchError::PoNotFound(_)),
        "expected PoNotFound, got: {:?}",
        err
    );
}

// ============================================================================
// 4. Match engine — two_way match with real PO + matching line
// ============================================================================

#[tokio::test]
#[serial]
async fn test_run_match_two_way() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let vendor_id = make_vendor(&pool, &tid).await;

    // Create PO with one line
    let po = create_po(
        &pool,
        &tid,
        &CreatePoRequest {
            vendor_id,
            currency: "USD".to_string(),
            created_by: "buyer-1".to_string(),
            expected_delivery_date: None,
            lines: vec![CreatePoLineRequest {
                item_id: None,
                description: Some("Consulting services".to_string()),
                quantity: 5.0,
                unit_of_measure: "each".to_string(),
                unit_price_minor: 10_000,
                gl_account_code: "6200".to_string(),
            }],
        },
        corr(),
    )
    .await
    .unwrap();

    let po_line_id = po.lines[0].line_id;

    // Create bill with line referencing the PO line
    let bill = create_bill(
        &pool,
        &tid,
        &CreateBillRequest {
            vendor_id,
            vendor_invoice_ref: "INV-MATCH2".to_string(),
            currency: "USD".to_string(),
            invoice_date: Utc::now(),
            due_date: None,
            tax_minor: None,
            entered_by: "clerk".to_string(),
            fx_rate_id: None,
            lines: vec![CreateBillLineRequest {
                description: Some("Consulting services".to_string()),
                item_id: None,
                quantity: 5.0,
                unit_price_minor: 10_000, // exact match
                gl_account_code: Some("6200".to_string()),
                po_line_id: Some(po_line_id),
            }],
        },
        corr(),
    )
    .await
    .unwrap();

    let outcome = run_match(
        &pool,
        &tid,
        bill.bill.bill_id,
        &RunMatchRequest {
            po_id: po.po.po_id,
            matched_by: "matcher-1".to_string(),
            price_tolerance_pct: 0.05,
        },
        corr(),
    )
    .await
    .unwrap();

    assert!(outcome.fully_matched);
    assert_eq!(outcome.lines.len(), 1);
    assert_eq!(outcome.lines[0].match_type, "two_way");
    assert!(outcome.lines[0].within_tolerance);

    // Bill status should now be "matched"
    let fetched = get_bill(&pool, &tid, bill.bill.bill_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fetched.bill.status, "matched");
}

// ============================================================================
// 5. Approve open bill (requires override_reason — unmatched)
// ============================================================================

#[tokio::test]
#[serial]
async fn test_approve_bill_with_override() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let vendor_id = make_vendor(&pool, &tid).await;

    let bill = create_bill(
        &pool,
        &tid,
        &make_bill_req(vendor_id, "INV-APPROVE"),
        corr(),
    )
    .await
    .unwrap();

    let approved = approve_bill(
        &pool,
        &ZeroTaxProvider,
        &tid,
        bill.bill.bill_id,
        &ApproveBillRequest {
            approved_by: "manager-1".to_string(),
            override_reason: Some("Spot purchase — PO waived".to_string()),
        },
        corr(),
    )
    .await
    .unwrap();

    assert_eq!(approved.status, "approved");
}

// ============================================================================
// 6. Void open bill
// ============================================================================

#[tokio::test]
#[serial]
async fn test_void_open_bill() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let vendor_id = make_vendor(&pool, &tid).await;

    let bill = create_bill(&pool, &tid, &make_bill_req(vendor_id, "INV-VOID"), corr())
        .await
        .unwrap();

    let voided = void_bill(
        &pool,
        &ZeroTaxProvider,
        &tid,
        bill.bill.bill_id,
        &VoidBillRequest {
            voided_by: "manager-1".to_string(),
            void_reason: "Entered in error".to_string(),
        },
        corr(),
    )
    .await
    .unwrap();

    assert_eq!(voided.status, "voided");
}

// ============================================================================
// 7. Void already-voided bill is idempotent (returns ok, no double-event)
// ============================================================================

#[tokio::test]
#[serial]
async fn test_void_idempotent() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let vendor_id = make_vendor(&pool, &tid).await;

    let bill = create_bill(&pool, &tid, &make_bill_req(vendor_id, "INV-VOID2"), corr())
        .await
        .unwrap();

    let void_req = VoidBillRequest {
        voided_by: "manager-1".to_string(),
        void_reason: "Test void".to_string(),
    };

    let first = void_bill(
        &pool,
        &ZeroTaxProvider,
        &tid,
        bill.bill.bill_id,
        &void_req,
        corr(),
    )
    .await
    .unwrap();

    // Second call is idempotent — returns ok with voided status, no error
    let second = void_bill(
        &pool,
        &ZeroTaxProvider,
        &tid,
        bill.bill.bill_id,
        &void_req,
        corr(),
    )
    .await
    .unwrap();

    assert_eq!(first.status, "voided");
    assert_eq!(second.status, "voided");
    assert_eq!(first.bill_id, second.bill_id);
}

// ============================================================================
// 8. Quote tax on a bill (ZeroTaxProvider → 0 tax)
// ============================================================================

#[tokio::test]
#[serial]
async fn test_quote_bill_tax() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let vendor_id = make_vendor(&pool, &tid).await;

    let bill = create_bill(&pool, &tid, &make_bill_req(vendor_id, "INV-TAX"), corr())
        .await
        .unwrap();

    let addr = TaxAddress {
        line1: "123 Main St".to_string(),
        line2: None,
        city: "San Francisco".to_string(),
        state: "CA".to_string(),
        postal_code: "94102".to_string(),
        country: "US".to_string(),
    };

    let snapshot = quote_bill_tax(
        &pool,
        &ZeroTaxProvider,
        "zero",
        &tid,
        bill.bill.bill_id,
        TaxQuoteRequest {
            tenant_id: tid.clone(),
            invoice_id: bill.bill.bill_id.to_string(),
            customer_id: vendor_id.to_string(),
            ship_to: addr.clone(),
            ship_from: addr,
            line_items: vec![TaxLineItem {
                line_id: bill.lines[0].line_id.to_string(),
                description: "Consulting services".to_string(),
                amount_minor: bill.lines[0].line_total_minor,
                currency: "USD".to_string(),
                tax_code: None,
                quantity: 5.0,
            }],
            currency: "USD".to_string(),
            invoice_date: Utc::now(),
            correlation_id: corr(),
        },
    )
    .await
    .unwrap();

    assert_eq!(snapshot.total_tax_minor, 0); // ZeroTaxProvider always returns 0
    assert_eq!(snapshot.status, "quoted");
}

// ============================================================================
// 9. Tenant isolation — cross-tenant bill access fails
// ============================================================================

#[tokio::test]
#[serial]
async fn test_tenant_isolation_bills() {
    let pool = setup_db().await;
    let tid_a = unique_tenant();
    let tid_b = unique_tenant();

    let vendor_id = make_vendor(&pool, &tid_a).await;
    let bill = create_bill(&pool, &tid_a, &make_bill_req(vendor_id, "INV-ISOL"), corr())
        .await
        .unwrap();

    let result = get_bill(&pool, &tid_b, bill.bill.bill_id).await.unwrap();
    assert!(result.is_none());
}
