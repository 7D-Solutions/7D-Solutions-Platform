//! Cross-module E2E: AP bill approval → GL posting + payment run
//!
//! Proves the full AP disbursement path:
//!   1. Create vendor in AP
//!   2. Create and approve a vendor bill in AP
//!   3. Read outbox event and feed it to GL consumer
//!   4. Verify balanced GL journal entry
//!   5. Execute payment run in AP → bill marked paid
//!
//! ## Prerequisites
//! - Docker containers running: `docker compose up -d`
//! - AP DB at localhost:5443, GL DB at localhost:5438

mod common;

use ap::domain::bills::approve::approve_bill;
use ap::domain::bills::service::create_bill;
use ap::domain::bills::{ApproveBillRequest, CreateBillLineRequest, CreateBillRequest};
use ap::domain::payment_runs::builder::create_payment_run;
use ap::domain::payment_runs::execute::execute_payment_run;
use ap::domain::payment_runs::CreatePaymentRunRequest;
use ap::domain::tax::ZeroTaxProvider;
use ap::domain::vendors::service::create_vendor;
use ap::domain::vendors::CreateVendorRequest;
use ap::events::EVENT_TYPE_VENDOR_BILL_APPROVED;
use chrono::Utc;
use common::{get_ap_pool, get_gl_pool};
use gl_rs::consumers::ap_vendor_bill_approved_consumer::{
    process_ap_bill_approved_posting, VendorBillApprovedPayload,
};
use serial_test::serial;
use uuid::Uuid;

const E2E_TENANT: &str = "e2e-ap-bill-payment-gl";

// ============================================================================
// GL setup helpers
// ============================================================================

async fn setup_gl(pool: &sqlx::PgPool) {
    let today = chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let period_end = chrono::NaiveDate::from_ymd_opt(2026, 12, 31).unwrap();

    let period_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO accounting_periods (id, tenant_id, period_start, period_end, is_closed, created_at)
         VALUES ($1, $2, $3, $4, false, NOW())",
    )
    .bind(period_id)
    .bind(E2E_TENANT)
    .bind(today)
    .bind(period_end)
    .execute(pool)
    .await
    .expect("Failed to create test period");

    for (code, name, acct_type, normal) in [
        ("AP", "Accounts Payable", "liability", "credit"),
        ("AP_CLEARING", "AP Clearing", "liability", "credit"),
        ("EXPENSE", "Default Expense", "expense", "debit"),
        ("6200", "Consulting Expense", "expense", "debit"),
    ] {
        let account_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO accounts (id, tenant_id, code, name, type, normal_balance, is_active, created_at)
             VALUES ($1, $2, $3, $4, $5::account_type, $6::normal_balance, true, NOW())",
        )
        .bind(account_id)
        .bind(E2E_TENANT)
        .bind(code)
        .bind(name)
        .bind(acct_type)
        .bind(normal)
        .execute(pool)
        .await
        .expect("Failed to create test account");
    }
}

/// Cleanup GL test data (FK-safe order)
async fn cleanup_gl(pool: &sqlx::PgPool) {
    for q in [
        "DELETE FROM processed_events WHERE tenant_id = $1",
        "DELETE FROM journal_lines WHERE journal_entry_id IN \
         (SELECT id FROM journal_entries WHERE tenant_id = $1)",
        "DELETE FROM journal_entries WHERE tenant_id = $1",
        "DELETE FROM account_balances WHERE tenant_id = $1",
        "DELETE FROM accounts WHERE tenant_id = $1",
        "DELETE FROM accounting_periods WHERE tenant_id = $1",
    ] {
        sqlx::query(q).bind(E2E_TENANT).execute(pool).await.ok();
    }
}

/// Cleanup AP test data (FK-safe order)
async fn cleanup_ap(pool: &sqlx::PgPool) {
    for q in [
        "DELETE FROM events_outbox WHERE aggregate_id IN \
         (SELECT bill_id::text FROM vendor_bills WHERE tenant_id = $1) \
         OR aggregate_id IN \
         (SELECT run_id::text FROM payment_runs WHERE tenant_id = $1)",
        "DELETE FROM payment_run_executions WHERE run_id IN \
         (SELECT run_id FROM payment_runs WHERE tenant_id = $1)",
        "DELETE FROM ap_allocations WHERE tenant_id = $1",
        "DELETE FROM payment_run_items WHERE run_id IN \
         (SELECT run_id FROM payment_runs WHERE tenant_id = $1)",
        "DELETE FROM payment_runs WHERE tenant_id = $1",
        "DELETE FROM ap_tax_snapshots WHERE tenant_id = $1",
        "DELETE FROM bill_match_results WHERE bill_id IN \
         (SELECT bill_id FROM vendor_bills WHERE tenant_id = $1)",
        "DELETE FROM bill_lines WHERE bill_id IN \
         (SELECT bill_id FROM vendor_bills WHERE tenant_id = $1)",
        "DELETE FROM vendor_bills WHERE tenant_id = $1",
        "DELETE FROM po_lines WHERE po_id IN \
         (SELECT po_id FROM purchase_orders WHERE tenant_id = $1)",
        "DELETE FROM purchase_orders WHERE tenant_id = $1",
        "DELETE FROM vendors WHERE tenant_id = $1",
    ] {
        sqlx::query(q).bind(E2E_TENANT).execute(pool).await.ok();
    }
}

// ============================================================================
// E2E Test: Full AP disbursement → GL posting
// ============================================================================

#[tokio::test]
#[serial]
async fn test_e2e_ap_bill_approve_triggers_gl_posting() {
    let gl_pool = get_gl_pool().await;
    let ap_db = get_ap_pool().await;

    // Clean slate
    cleanup_gl(&gl_pool).await;
    cleanup_ap(&ap_db).await;

    // -- GL setup: accounting period + chart of accounts
    setup_gl(&gl_pool).await;

    // -- Step 1: Create vendor in AP
    let vendor = create_vendor(
        &ap_db,
        E2E_TENANT,
        &CreateVendorRequest {
            name: "E2E Test Vendor".to_string(),
            tax_id: None,
            currency: "USD".to_string(),
            payment_terms_days: 0,
            payment_method: Some("ach".to_string()),
            remittance_email: None,
            party_id: None,
        },
        Uuid::new_v4().to_string(),
    )
    .await
    .expect("create vendor");

    // -- Step 2: Create bill in AP
    let bill = create_bill(
        &ap_db,
        E2E_TENANT,
        &CreateBillRequest {
            vendor_id: vendor.vendor_id,
            vendor_invoice_ref: "E2E-INV-001".to_string(),
            currency: "USD".to_string(),
            invoice_date: Utc::now(),
            due_date: None,
            tax_minor: None,
            entered_by: "e2e-clerk".to_string(),
            fx_rate_id: None,
            lines: vec![CreateBillLineRequest {
                description: Some("Consulting services".to_string()),
                item_id: None,
                quantity: 2.0,
                unit_price_minor: 25_000, // $250.00 per unit
                gl_account_code: Some("6200".to_string()),
                po_line_id: None,
            }],
        },
        Uuid::new_v4().to_string(),
    )
    .await
    .expect("create bill");

    assert_eq!(bill.bill.status, "open");
    assert_eq!(bill.bill.total_minor, 50_000); // 2 × $250

    // -- Step 3: Approve bill (generates outbox event)
    let approved = approve_bill(
        &ap_db,
        &ZeroTaxProvider,
        E2E_TENANT,
        bill.bill.bill_id,
        &ApproveBillRequest {
            approved_by: "e2e-manager".to_string(),
            override_reason: Some("E2E test — no PO required".to_string()),
        },
        Uuid::new_v4().to_string(),
    )
    .await
    .expect("approve bill");

    assert_eq!(approved.status, "approved");

    // -- Step 4: Read the outbox event from AP and extract payload
    let outbox_row: (Uuid, serde_json::Value) = sqlx::query_as(
        "SELECT event_id, payload FROM events_outbox \
         WHERE event_type = $1 AND aggregate_id = $2 \
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(EVENT_TYPE_VENDOR_BILL_APPROVED)
    .bind(bill.bill.bill_id.to_string())
    .fetch_one(&ap_db)
    .await
    .expect("outbox event must exist after approval");

    let (outbox_event_id, outbox_payload) = outbox_row;

    // Parse the nested payload (envelope.payload contains VendorBillApprovedPayload)
    let gl_payload: VendorBillApprovedPayload =
        serde_json::from_value(outbox_payload["payload"].clone())
            .expect("deserialize VendorBillApprovedPayload from outbox");

    assert_eq!(gl_payload.bill_id, bill.bill.bill_id);
    assert_eq!(gl_payload.approved_amount_minor, 50_000);
    assert_eq!(gl_payload.currency, "USD");
    assert!(
        !gl_payload.gl_lines.is_empty(),
        "gl_lines must be populated"
    );

    // -- Step 5: Feed event to GL consumer (simulates NATS delivery)
    let journal_entry_id =
        process_ap_bill_approved_posting(&gl_pool, outbox_event_id, E2E_TENANT, "ap", &gl_payload)
            .await
            .expect("GL posting should succeed");

    // -- Step 6: Verify GL journal entry exists and is balanced
    let (je_count,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM journal_entries WHERE id = $1 AND tenant_id = $2")
            .bind(journal_entry_id)
            .bind(E2E_TENANT)
            .fetch_one(&gl_pool)
            .await
            .expect("query journal_entries");
    assert_eq!(je_count, 1, "journal entry must exist in GL");

    // Verify balanced: total debits == total credits
    let (total_dr, total_cr): (i64, i64) = sqlx::query_as(
        "SELECT SUM(debit_minor)::bigint, SUM(credit_minor)::bigint \
         FROM journal_lines WHERE journal_entry_id = $1",
    )
    .bind(journal_entry_id)
    .fetch_one(&gl_pool)
    .await
    .expect("sum journal_lines");
    assert_eq!(total_dr, total_cr, "GL journal must balance (DR = CR)");
    assert_eq!(total_dr, 50_000, "debit total must match bill amount");

    // Verify line count: 1 debit (expense) + 1 credit (AP) = 2 lines
    let (line_count,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM journal_lines WHERE journal_entry_id = $1")
            .bind(journal_entry_id)
            .fetch_one(&gl_pool)
            .await
            .expect("count lines");
    assert_eq!(line_count, 2, "one debit + one credit line");

    // Verify debit goes to expense account 6200
    let (debit_account,): (String,) = sqlx::query_as(
        "SELECT account_ref FROM journal_lines \
         WHERE journal_entry_id = $1 AND debit_minor > 0",
    )
    .bind(journal_entry_id)
    .fetch_one(&gl_pool)
    .await
    .expect("debit line");
    assert_eq!(debit_account, "6200", "debit must post to expense account");

    // Verify credit goes to AP liability
    let (credit_account,): (String,) = sqlx::query_as(
        "SELECT account_ref FROM journal_lines \
         WHERE journal_entry_id = $1 AND credit_minor > 0",
    )
    .bind(journal_entry_id)
    .fetch_one(&gl_pool)
    .await
    .expect("credit line");
    assert_eq!(credit_account, "AP", "credit must post to AP liability");

    // -- Cleanup
    cleanup_gl(&gl_pool).await;
    cleanup_ap(&ap_db).await;
    gl_pool.close().await;
    ap_db.close().await;
}

// ============================================================================
// E2E Test: Full path including payment run execution
// ============================================================================

#[tokio::test]
#[serial]
async fn test_e2e_ap_payment_run_marks_bill_paid_after_gl_posting() {
    let gl_pool = get_gl_pool().await;
    let ap_db = get_ap_pool().await;

    cleanup_gl(&gl_pool).await;
    cleanup_ap(&ap_db).await;
    setup_gl(&gl_pool).await;

    // -- Create vendor + bill + approve
    let vendor = create_vendor(
        &ap_db,
        E2E_TENANT,
        &CreateVendorRequest {
            name: "E2E Payment Vendor".to_string(),
            tax_id: None,
            currency: "USD".to_string(),
            payment_terms_days: 0,
            payment_method: Some("ach".to_string()),
            remittance_email: None,
            party_id: None,
        },
        Uuid::new_v4().to_string(),
    )
    .await
    .expect("create vendor");

    let bill = create_bill(
        &ap_db,
        E2E_TENANT,
        &CreateBillRequest {
            vendor_id: vendor.vendor_id,
            vendor_invoice_ref: "E2E-PAY-001".to_string(),
            currency: "USD".to_string(),
            invoice_date: Utc::now(),
            due_date: None,
            tax_minor: None,
            entered_by: "e2e-clerk".to_string(),
            fx_rate_id: None,
            lines: vec![CreateBillLineRequest {
                description: Some("Server hosting".to_string()),
                item_id: None,
                quantity: 1.0,
                unit_price_minor: 75_000, // $750.00
                gl_account_code: Some("6200".to_string()),
                po_line_id: None,
            }],
        },
        Uuid::new_v4().to_string(),
    )
    .await
    .expect("create bill");

    approve_bill(
        &ap_db,
        &ZeroTaxProvider,
        E2E_TENANT,
        bill.bill.bill_id,
        &ApproveBillRequest {
            approved_by: "e2e-cfo".to_string(),
            override_reason: Some("E2E test".to_string()),
        },
        Uuid::new_v4().to_string(),
    )
    .await
    .expect("approve bill");

    // -- Read outbox and post to GL
    let outbox_row: (Uuid, serde_json::Value) = sqlx::query_as(
        "SELECT event_id, payload FROM events_outbox \
         WHERE event_type = $1 AND aggregate_id = $2 \
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(EVENT_TYPE_VENDOR_BILL_APPROVED)
    .bind(bill.bill.bill_id.to_string())
    .fetch_one(&ap_db)
    .await
    .expect("outbox event");

    let gl_payload: VendorBillApprovedPayload =
        serde_json::from_value(outbox_row.1["payload"].clone()).expect("deserialize payload");

    let je_id =
        process_ap_bill_approved_posting(&gl_pool, outbox_row.0, E2E_TENANT, "ap", &gl_payload)
            .await
            .expect("GL posting");

    // Verify GL posting succeeded
    let (je_exists,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM journal_entries WHERE id = $1 AND tenant_id = $2")
            .bind(je_id)
            .bind(E2E_TENANT)
            .fetch_one(&gl_pool)
            .await
            .expect("verify JE");
    assert_eq!(je_exists, 1);

    // -- Execute payment run
    let run = create_payment_run(
        &ap_db,
        E2E_TENANT,
        &CreatePaymentRunRequest {
            run_id: Uuid::new_v4(),
            currency: "USD".to_string(),
            scheduled_date: Utc::now(),
            payment_method: "ach".to_string(),
            created_by: "e2e-treasurer".to_string(),
            due_on_or_before: None,
            vendor_ids: None,
            correlation_id: Some(Uuid::new_v4().to_string()),
        },
    )
    .await
    .expect("create payment run");

    assert_eq!(run.run.status, "pending");
    assert!(!run.items.is_empty(), "run must include approved bill");

    let result = execute_payment_run(&ap_db, E2E_TENANT, run.run.run_id)
        .await
        .expect("execute payment run");

    assert_eq!(result.run.status, "completed");
    assert!(!result.executions.is_empty());
    assert!(result.run.executed_at.is_some());

    // -- Verify bill is marked paid in AP
    let (bill_status,): (String,) =
        sqlx::query_as("SELECT status FROM vendor_bills WHERE bill_id = $1 AND tenant_id = $2")
            .bind(bill.bill.bill_id)
            .bind(E2E_TENANT)
            .fetch_one(&ap_db)
            .await
            .expect("fetch bill status");
    assert_eq!(bill_status, "paid", "bill must be paid after payment run");

    // -- Verify ap.payment_executed event in outbox
    let (pay_event_count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM events_outbox \
         WHERE event_type = 'ap.payment_executed' AND aggregate_id = $1",
    )
    .bind(run.run.run_id.to_string())
    .fetch_one(&ap_db)
    .await
    .expect("payment event");
    assert_eq!(pay_event_count, 1, "ap.payment_executed must be in outbox");

    // -- Cleanup
    cleanup_gl(&gl_pool).await;
    cleanup_ap(&ap_db).await;
    gl_pool.close().await;
    ap_db.close().await;
}
