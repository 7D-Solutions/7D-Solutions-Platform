//! Integration tests for AP payment runs: create + execute (bd-3rvu).
//!
//! Covers:
//! 1. Create payment run — happy path (eligible approved bill)
//! 2. Create payment run — no eligible bills → NoBillsEligible
//! 3. Execute payment run → bills paid, status completed
//! 4. Create payment run idempotent (same run_id twice)

use ap::domain::bills::approve::approve_bill;
use ap::domain::bills::service::create_bill;
use ap::domain::bills::{ApproveBillRequest, CreateBillLineRequest, CreateBillRequest};
use ap::domain::payment_runs::builder::create_payment_run;
use ap::domain::payment_runs::execute::execute_payment_run;
use ap::domain::payment_runs::{CreatePaymentRunRequest, PaymentRunError};
use ap::domain::tax::ZeroTaxProvider;
use ap::domain::vendors::service::create_vendor;
use ap::domain::vendors::CreateVendorRequest;
use chrono::Utc;
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
    format!("ap-run-{}", Uuid::new_v4().simple())
}

async fn make_vendor(pool: &sqlx::PgPool, tid: &str) -> Uuid {
    create_vendor(
        pool,
        tid,
        &CreateVendorRequest {
            name: format!("Vendor-{}", Uuid::new_v4().simple()),
            tax_id: None,
            currency: "USD".to_string(),
            payment_terms_days: 0, // Net0 — bill immediately due
            payment_method: Some("ach".to_string()),
            remittance_email: None,
            party_id: None,
        },
        Uuid::new_v4().to_string(),
    )
    .await
    .unwrap()
    .vendor_id
}

/// Create a vendor bill and immediately approve it (spot purchase override).
async fn make_approved_bill(
    pool: &sqlx::PgPool,
    tid: &str,
    vendor_id: Uuid,
    ref_num: &str,
) -> Uuid {
    let bill = create_bill(
        pool,
        tid,
        &CreateBillRequest {
            vendor_id,
            vendor_invoice_ref: ref_num.to_string(),
            currency: "USD".to_string(),
            invoice_date: Utc::now(),
            due_date: None,
            tax_minor: None,
            entered_by: "clerk".to_string(),
            fx_rate_id: None,
            lines: vec![CreateBillLineRequest {
                description: Some("Service".to_string()),
                item_id: None,
                quantity: 1.0,
                unit_price_minor: 10_000,
                gl_account_code: Some("6200".to_string()),
                po_line_id: None,
            }],
        },
        Uuid::new_v4().to_string(),
    )
    .await
    .unwrap();

    approve_bill(
        pool,
        &ZeroTaxProvider,
        tid,
        bill.bill.bill_id,
        &ApproveBillRequest {
            approved_by: "manager".to_string(),
            override_reason: Some("Spot purchase".to_string()),
        },
        Uuid::new_v4().to_string(),
    )
    .await
    .unwrap();

    bill.bill.bill_id
}

fn base_run_req(run_id: Uuid) -> CreatePaymentRunRequest {
    CreatePaymentRunRequest {
        run_id,
        currency: "USD".to_string(),
        scheduled_date: Utc::now(),
        payment_method: "ach".to_string(),
        created_by: "treasurer".to_string(),
        due_on_or_before: None,
        vendor_ids: None,
        correlation_id: Some(Uuid::new_v4().to_string()),
    }
}

// ============================================================================
// 1. Create payment run — happy path
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_payment_run() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let vendor_id = make_vendor(&pool, &tid).await;

    make_approved_bill(&pool, &tid, vendor_id, "INV-PR-001").await;

    let result = create_payment_run(&pool, &tid, &base_run_req(Uuid::new_v4()))
        .await
        .unwrap();

    assert_eq!(result.run.status, "pending");
    assert!(!result.items.is_empty());
    assert!(result.run.total_minor > 0);
}

// ============================================================================
// 2. Create payment run — no eligible bills → NoBillsEligible
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_payment_run_no_eligible_bills() {
    let pool = setup_db().await;
    let tid = unique_tenant(); // fresh tenant — no approved bills

    let err = create_payment_run(&pool, &tid, &base_run_req(Uuid::new_v4()))
        .await
        .unwrap_err();

    assert!(
        matches!(err, PaymentRunError::NoBillsEligible(_, _)),
        "expected NoBillsEligible, got: {:?}",
        err
    );
}

// ============================================================================
// 3. Execute payment run → bills marked paid, run completed
// ============================================================================

#[tokio::test]
#[serial]
async fn test_execute_payment_run() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let vendor_id = make_vendor(&pool, &tid).await;

    make_approved_bill(&pool, &tid, vendor_id, "INV-EXEC-001").await;

    let run = create_payment_run(&pool, &tid, &base_run_req(Uuid::new_v4()))
        .await
        .unwrap();

    assert_eq!(run.run.status, "pending");

    let result = execute_payment_run(&pool, &tid, run.run.run_id)
        .await
        .unwrap();

    assert_eq!(result.run.status, "completed");
    assert!(!result.executions.is_empty());
    assert!(result.run.executed_at.is_some());
}

// ============================================================================
// 4. Create payment run idempotent (same run_id twice → same run returned)
// ============================================================================

#[tokio::test]
#[serial]
async fn test_payment_run_create_idempotent() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let vendor_id = make_vendor(&pool, &tid).await;

    make_approved_bill(&pool, &tid, vendor_id, "INV-IDEM-001").await;

    let run_id = Uuid::new_v4();
    let req = base_run_req(run_id);

    let first = create_payment_run(&pool, &tid, &req).await.unwrap();
    let second = create_payment_run(&pool, &tid, &req).await.unwrap();

    assert_eq!(first.run.run_id, second.run.run_id);
    assert_eq!(first.run.total_minor, second.run.total_minor);
    assert_eq!(first.items.len(), second.items.len());
}
