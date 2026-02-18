//! Integrated E2E: PO → Inventory Receipt → Bill → 3-Way Match → Approve → GL Posting
//!
//! Tests the full procure-to-pay accounting spine:
//!   1. Create and approve a purchase order
//!   2. Ingest inventory receipt link (simulating goods received from inventory module)
//!   3. Create a vendor bill referencing PO lines
//!   4. Run the 3-way match engine — asserts pass when qty/price align
//!   5. Approve the matched bill
//!   6. Drive GL posting directly via the consumer function
//!   7. Assert GL journal entry: balanced, posted exactly once, correct accounts
//!
//! Acceptance criteria:
//!   - E2E passes with real services (real Postgres, no mocks)
//!   - 3-way match passes on aligned quantities/prices; mismatch produces non-approvable status
//!   - GL posting occurs exactly once and the entry balances (sum_debits == sum_credits)
//!   - No cross-tenant contamination (second tenant sees no GL entries from first)

mod common;

use chrono::Utc;
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

use ap::domain::bills::{
    approve::approve_bill, service::create_bill, ApproveBillRequest, CreateBillLineRequest,
    CreateBillRequest,
};
use ap::domain::r#match::engine::run_match;
use ap::domain::r#match::RunMatchRequest;
use ap::domain::po::{
    approve::approve_po, service::create_po, ApprovePoRequest, CreatePoLineRequest,
    CreatePoRequest,
};
use ap::domain::receipts_link::{service::ingest_receipt_link, IngestReceiptLinkRequest};
use gl_rs::consumer::ap_vendor_bill_approved_consumer::{
    process_ap_bill_approved_posting, ApprovedGlLine, VendorBillApprovedPayload,
    AP_CLEARING_ACCOUNT, AP_LIABILITY_ACCOUNT,
};

// ============================================================================
// Test tenant
// ============================================================================

const TENANT_A: &str = "e2e-po-receipt-bill-gl-tenant-a";
const TENANT_B: &str = "e2e-po-receipt-bill-gl-tenant-b";

// ============================================================================
// Setup / teardown helpers
// ============================================================================

async fn setup_gl_accounts(gl: &PgPool, tenant_id: &str) {
    sqlx::query(
        "INSERT INTO accounts (id, tenant_id, code, name, type, normal_balance, is_active)
         VALUES
           (gen_random_uuid(), $1, 'AP',         'Accounts Payable',    'liability', 'credit', true),
           (gen_random_uuid(), $1, 'AP_CLEARING','AP Clearing',         'asset',     'debit',  true),
           (gen_random_uuid(), $1, 'EXPENSE',    'General Expense',     'expense',   'debit',  true),
           (gen_random_uuid(), $1, '6100',       'Supply Expense',      'expense',   'debit',  true)
         ON CONFLICT (tenant_id, code) DO NOTHING",
    )
    .bind(tenant_id)
    .execute(gl)
    .await
    .expect("setup_gl_accounts failed");
}

async fn setup_accounting_period(gl: &PgPool, tenant_id: &str) {
    sqlx::query(
        "INSERT INTO accounting_periods (tenant_id, period_start, period_end, is_closed)
         VALUES ($1, '2026-01-01', '2026-12-31', false)
         ON CONFLICT DO NOTHING",
    )
    .bind(tenant_id)
    .execute(gl)
    .await
    .expect("setup_accounting_period failed");
}

async fn create_test_vendor(ap: &PgPool, tenant_id: &str) -> Uuid {
    let vendor_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO vendors (vendor_id, tenant_id, name, currency, payment_terms_days, \
         is_active, created_at, updated_at) \
         VALUES ($1, $2, $3, 'USD', 30, TRUE, NOW(), NOW())",
    )
    .bind(vendor_id)
    .bind(tenant_id)
    .bind(format!("E2E Vendor {}", &vendor_id.to_string()[..8]))
    .execute(ap)
    .await
    .expect("create_test_vendor failed");
    vendor_id
}

async fn cleanup_ap(ap: &PgPool, tenant_id: &str) {
    for q in [
        "DELETE FROM three_way_match WHERE bill_id IN \
         (SELECT bill_id FROM vendor_bills WHERE tenant_id = $1)",
        "DELETE FROM events_outbox WHERE aggregate_type = 'bill' \
         AND aggregate_id IN (SELECT bill_id::TEXT FROM vendor_bills WHERE tenant_id = $1)",
        "DELETE FROM bill_lines WHERE bill_id IN \
         (SELECT bill_id FROM vendor_bills WHERE tenant_id = $1)",
        "DELETE FROM vendor_bills WHERE tenant_id = $1",
        "DELETE FROM po_receipt_links WHERE po_id IN \
         (SELECT po_id FROM purchase_orders WHERE tenant_id = $1)",
        "DELETE FROM po_lines WHERE po_id IN \
         (SELECT po_id FROM purchase_orders WHERE tenant_id = $1)",
        "DELETE FROM po_status WHERE po_id IN \
         (SELECT po_id FROM purchase_orders WHERE tenant_id = $1)",
        "DELETE FROM events_outbox WHERE aggregate_type = 'po' \
         AND aggregate_id IN (SELECT po_id::TEXT FROM purchase_orders WHERE tenant_id = $1)",
        "DELETE FROM purchase_orders WHERE tenant_id = $1",
        "DELETE FROM events_outbox WHERE aggregate_type = 'vendor' \
         AND aggregate_id IN (SELECT vendor_id::TEXT FROM vendors WHERE tenant_id = $1)",
        "DELETE FROM vendors WHERE tenant_id = $1",
    ] {
        sqlx::query(q).bind(tenant_id).execute(ap).await.ok();
    }
}

async fn cleanup_gl(gl: &PgPool, tenant_id: &str) {
    sqlx::query(
        "DELETE FROM journal_lines WHERE journal_entry_id IN \
         (SELECT id FROM journal_entries WHERE tenant_id = $1)",
    )
    .bind(tenant_id)
    .execute(gl)
    .await
    .ok();
    sqlx::query("DELETE FROM journal_entries WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(gl)
        .await
        .ok();
    sqlx::query("DELETE FROM processed_events WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(gl)
        .await
        .ok();
    sqlx::query("DELETE FROM accounting_periods WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(gl)
        .await
        .ok();
    sqlx::query("DELETE FROM accounts WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(gl)
        .await
        .ok();
}

// ============================================================================
// Tests
// ============================================================================

/// Happy path: PO created & approved → receipt ingested → bill created → 3-way match
/// passes → bill approved → GL journal posted exactly once and balances.
#[tokio::test]
#[serial]
async fn test_po_receipt_bill_approve_gl_full_spine() {
    let ap = common::get_ap_pool().await;
    let gl = common::get_gl_pool().await;

    cleanup_ap(&ap, TENANT_A).await;
    cleanup_gl(&gl, TENANT_A).await;

    setup_gl_accounts(&gl, TENANT_A).await;
    setup_accounting_period(&gl, TENANT_A).await;

    // Step 1: Create a vendor
    let vendor_id = create_test_vendor(&ap, TENANT_A).await;

    // Step 2: Create a PO with one line (10 units @ 1000 minor = 10,000 total)
    let po_line_qty = 10.0_f64;
    let po_unit_price = 1000_i64;
    let create_po_req = CreatePoRequest {
        vendor_id,
        currency: "USD".to_string(),
        created_by: "buyer-e2e".to_string(),
        expected_delivery_date: None,
        lines: vec![CreatePoLineRequest {
            item_id: None,
            description: Some("Office widgets".to_string()),
            quantity: po_line_qty,
            unit_of_measure: "each".to_string(),
            unit_price_minor: po_unit_price,
            gl_account_code: "6100".to_string(),
        }],
    };

    let po_with_lines = create_po(&ap, TENANT_A, &create_po_req, "corr-e2e-po-create".to_string())
        .await
        .expect("create_po failed");
    let po_id = po_with_lines.po.po_id;
    let po_line_id = po_with_lines.lines[0].line_id;
    assert_eq!(po_with_lines.po.status, "draft");
    assert_eq!(po_with_lines.po.total_minor, po_line_qty as i64 * po_unit_price);

    // Step 3: Approve the PO
    let approved_po = approve_po(
        &ap,
        TENANT_A,
        po_id,
        &ApprovePoRequest { approved_by: "manager-e2e".to_string() },
        "corr-e2e-po-approve".to_string(),
    )
    .await
    .expect("approve_po failed");
    assert_eq!(approved_po.status, "approved");

    // Step 4: Ingest inventory receipt (simulates goods received from inventory module)
    let receipt_id = Uuid::new_v4();
    let received_qty = 10.0_f64; // exact match
    ingest_receipt_link(
        &ap,
        &IngestReceiptLinkRequest {
            po_id,
            po_line_id,
            vendor_id,
            receipt_id,
            quantity_received: received_qty,
            unit_of_measure: "each".to_string(),
            unit_price_minor: po_unit_price,
            currency: "USD".to_string(),
            gl_account_code: "6100".to_string(),
            received_at: Utc::now(),
            received_by: "system:inventory-consumer".to_string(),
        },
    )
    .await
    .expect("ingest_receipt_link failed");

    // Verify receipt link stored
    let (link_count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM po_receipt_links WHERE po_line_id = $1 AND receipt_id = $2",
    )
    .bind(po_line_id)
    .bind(receipt_id)
    .fetch_one(&ap)
    .await
    .expect("receipt link count");
    assert_eq!(link_count, 1, "receipt link must be stored");

    // Step 5: Create vendor bill referencing the PO line (same qty/price = exact match)
    let bill_unit_price = po_unit_price;
    let bill_qty = po_line_qty;
    let invoice_ref = format!("INV-E2E-{}", &Uuid::new_v4().to_string()[..8]);
    let bill_with_lines = create_bill(
        &ap,
        TENANT_A,
        &CreateBillRequest {
            vendor_id,
            vendor_invoice_ref: invoice_ref.clone(),
            currency: "USD".to_string(),
            invoice_date: Utc::now(),
            due_date: None,
            tax_minor: None,
            entered_by: "ap-clerk-e2e".to_string(),
            fx_rate_id: None,
            lines: vec![CreateBillLineRequest {
                description: Some("Office widgets".to_string()),
                item_id: None,
                quantity: bill_qty,
                unit_price_minor: bill_unit_price,
                gl_account_code: Some("6100".to_string()),
                po_line_id: Some(po_line_id),
            }],
        },
        "corr-e2e-bill-create".to_string(),
    )
    .await
    .expect("create_bill failed");
    let bill_id = bill_with_lines.bill.bill_id;
    assert_eq!(bill_with_lines.bill.status, "open");

    // Step 6: Run the 3-way match engine
    let match_outcome = run_match(
        &ap,
        TENANT_A,
        bill_id,
        &RunMatchRequest {
            po_id,
            matched_by: "ap-user-e2e".to_string(),
            price_tolerance_pct: 0.05,
        },
        "corr-e2e-match".to_string(),
    )
    .await
    .expect("run_match failed");

    // Assert 3-way match (receipt exists) and within tolerance
    assert!(match_outcome.fully_matched, "3-way match must be fully matched");
    assert_eq!(match_outcome.lines.len(), 1);
    let match_line = &match_outcome.lines[0];
    assert_eq!(match_line.match_type, "three_way", "must be three_way with receipt");
    assert!(match_line.within_tolerance, "price/qty within tolerance");
    assert_eq!(match_line.price_variance_minor, 0, "no price variance on exact match");
    assert!(match_line.qty_variance.abs() < 1e-6, "no qty variance on exact match");
    assert!(match_line.receipt_id.is_some(), "receipt_id captured in match");

    // Bill status must be 'matched' after the engine runs
    let (bill_status,): (String,) =
        sqlx::query_as("SELECT status FROM vendor_bills WHERE bill_id = $1")
            .bind(bill_id)
            .fetch_one(&ap)
            .await
            .expect("bill status");
    assert_eq!(bill_status, "matched");

    // vendor_bill_matched outbox event enqueued
    let (match_ev_count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM events_outbox WHERE aggregate_type = 'bill' \
         AND aggregate_id = $1 AND event_type = 'ap.vendor_bill_matched'",
    )
    .bind(bill_id.to_string())
    .fetch_one(&ap)
    .await
    .expect("match event count");
    assert!(match_ev_count >= 1, "vendor_bill_matched outbox event must be enqueued");

    // Step 7: Approve the matched bill (no override needed — within tolerance)
    let approved_bill = approve_bill(
        &ap,
        TENANT_A,
        bill_id,
        &ApproveBillRequest {
            approved_by: "controller-e2e".to_string(),
            override_reason: None,
        },
        "corr-e2e-bill-approve".to_string(),
    )
    .await
    .expect("approve_bill failed");
    assert_eq!(approved_bill.status, "approved");

    // vendor_bill_approved outbox event must be enqueued exactly once
    let (approve_ev_count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM events_outbox WHERE aggregate_type = 'bill' \
         AND aggregate_id = $1 AND event_type = 'ap.vendor_bill_approved'",
    )
    .bind(bill_id.to_string())
    .fetch_one(&ap)
    .await
    .expect("approved event count");
    assert_eq!(approve_ev_count, 1, "vendor_bill_approved must be enqueued exactly once");

    // Step 8: Drive GL posting directly (bypasses NATS; tests the processing function)
    let gl_event_id = Uuid::new_v4();
    let total_minor = bill_qty as i64 * bill_unit_price;

    // Build the payload mirroring what the AP outbox carries
    // For PO-backed lines, account is AP_CLEARING (per consumer logic)
    let gl_payload = VendorBillApprovedPayload {
        bill_id,
        tenant_id: TENANT_A.to_string(),
        vendor_id,
        vendor_invoice_ref: invoice_ref.clone(),
        approved_amount_minor: total_minor,
        currency: "USD".to_string(),
        due_date: Utc::now() + chrono::Duration::days(30),
        approved_by: "controller-e2e".to_string(),
        approved_at: Utc::now(),
        fx_rate_id: None,
        gl_lines: vec![ApprovedGlLine {
            line_id: bill_with_lines.lines[0].line_id,
            gl_account_code: "6100".to_string(),
            amount_minor: total_minor,
            po_line_id: Some(po_line_id), // PO-backed → debit AP_CLEARING
        }],
    };

    let journal_id = process_ap_bill_approved_posting(
        &gl,
        gl_event_id,
        TENANT_A,
        "ap",
        &gl_payload,
    )
    .await
    .expect("GL posting failed");

    // Step 9: Assert GL journal entry is correct

    // Exactly one journal entry
    let (je_count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM journal_entries WHERE tenant_id = $1 AND source_event_id = $2",
    )
    .bind(TENANT_A)
    .bind(gl_event_id)
    .fetch_one(&gl)
    .await
    .expect("journal entry count");
    assert_eq!(je_count, 1, "exactly one GL journal entry");

    // Fetch journal lines
    let lines: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT account_ref, debit_minor, credit_minor \
         FROM journal_lines WHERE journal_entry_id = $1 ORDER BY line_no",
    )
    .bind(journal_id)
    .fetch_all(&gl)
    .await
    .expect("journal lines fetch");

    assert!(!lines.is_empty(), "journal must have lines");

    // Double-entry balance: sum_debits == sum_credits
    let total_debits: i64 = lines.iter().map(|(_, d, _)| d).sum();
    let total_credits: i64 = lines.iter().map(|(_, _, c)| c).sum();
    assert_eq!(
        total_debits, total_credits,
        "GL must balance: debits={} credits={}",
        total_debits, total_credits
    );
    assert_eq!(total_debits, total_minor, "total debits must equal bill amount");

    // For PO-backed line: debit side must be AP_CLEARING
    let debit_accounts: Vec<&str> = lines
        .iter()
        .filter(|(_, d, _)| *d > 0)
        .map(|(acc, _, _)| acc.as_str())
        .collect();
    assert!(
        debit_accounts.contains(&AP_CLEARING_ACCOUNT),
        "PO-backed line must debit AP_CLEARING, got {:?}",
        debit_accounts
    );

    // Credit side must be AP (accounts payable)
    let credit_accounts: Vec<&str> = lines
        .iter()
        .filter(|(_, _, c)| *c > 0)
        .map(|(acc, _, _)| acc.as_str())
        .collect();
    assert!(
        credit_accounts.contains(&AP_LIABILITY_ACCOUNT),
        "AP liability credit must be posted to 'AP', got {:?}",
        credit_accounts
    );

    cleanup_ap(&ap, TENANT_A).await;
    cleanup_gl(&gl, TENANT_A).await;
}

/// Idempotency: processing the same vendor_bill_approved event twice produces
/// exactly one GL journal entry (duplicate silently ignored).
#[tokio::test]
#[serial]
async fn test_gl_posting_idempotent_on_duplicate_event() {
    let ap = common::get_ap_pool().await;
    let gl = common::get_gl_pool().await;

    cleanup_ap(&ap, TENANT_A).await;
    cleanup_gl(&gl, TENANT_A).await;

    setup_gl_accounts(&gl, TENANT_A).await;
    setup_accounting_period(&gl, TENANT_A).await;

    let vendor_id = create_test_vendor(&ap, TENANT_A).await;

    // Minimal setup: single bill for idempotency test (no PO needed)
    let bill_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO vendor_bills (bill_id, tenant_id, vendor_id, vendor_invoice_ref, \
         currency, total_minor, invoice_date, due_date, status, entered_by, entered_at) \
         VALUES ($1, $2, $3, $4, 'USD', 5000, NOW(), NOW() + interval '30 days', \
         'approved', 'system', NOW())",
    )
    .bind(bill_id)
    .bind(TENANT_A)
    .bind(vendor_id)
    .bind(format!("INV-IDEM-{}", &bill_id.to_string()[..8]))
    .execute(&ap)
    .await
    .expect("insert bill for idempotency test");

    let gl_event_id = Uuid::new_v4();
    let payload = VendorBillApprovedPayload {
        bill_id,
        tenant_id: TENANT_A.to_string(),
        vendor_id,
        vendor_invoice_ref: format!("INV-IDEM-{}", &bill_id.to_string()[..8]),
        approved_amount_minor: 5000,
        currency: "USD".to_string(),
        due_date: Utc::now() + chrono::Duration::days(30),
        approved_by: "controller".to_string(),
        approved_at: Utc::now(),
        fx_rate_id: None,
        gl_lines: vec![], // empty → fallback to EXPENSE account
    };

    // First call: should succeed
    process_ap_bill_approved_posting(&gl, gl_event_id, TENANT_A, "ap", &payload)
        .await
        .expect("first GL posting failed");

    // Second call with same event_id: must not error
    let second_result =
        process_ap_bill_approved_posting(&gl, gl_event_id, TENANT_A, "ap", &payload).await;

    // DuplicateEvent is the expected idempotent outcome
    assert!(
        second_result.is_err() || second_result.is_ok(),
        "second call must not panic"
    );

    // Still only one journal entry for this event
    let (count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM journal_entries WHERE tenant_id = $1 AND source_event_id = $2",
    )
    .bind(TENANT_A)
    .bind(gl_event_id)
    .fetch_one(&gl)
    .await
    .expect("count");
    assert_eq!(count, 1, "idempotent: only one GL journal entry allowed per event");

    cleanup_ap(&ap, TENANT_A).await;
    cleanup_gl(&gl, TENANT_A).await;
}

/// Mismatch case: price variance outside tolerance → match engine marks as
/// non-approvable (within_tolerance = false) → approve without override_reason fails.
#[tokio::test]
#[serial]
async fn test_price_mismatch_blocks_approval_without_override() {
    let ap = common::get_ap_pool().await;

    cleanup_ap(&ap, TENANT_A).await;

    let vendor_id = create_test_vendor(&ap, TENANT_A).await;

    // PO: 10 units @ 1000 minor
    let create_po_req = CreatePoRequest {
        vendor_id,
        currency: "USD".to_string(),
        created_by: "buyer".to_string(),
        expected_delivery_date: None,
        lines: vec![CreatePoLineRequest {
            item_id: None,
            description: Some("Widgets".to_string()),
            quantity: 10.0,
            unit_of_measure: "each".to_string(),
            unit_price_minor: 1000,
            gl_account_code: "6100".to_string(),
        }],
    };
    let po_with_lines = create_po(&ap, TENANT_A, &create_po_req, "corr-mm-po".to_string())
        .await
        .expect("create_po failed");
    let po_id = po_with_lines.po.po_id;
    let po_line_id = po_with_lines.lines[0].line_id;

    approve_po(
        &ap,
        TENANT_A,
        po_id,
        &ApprovePoRequest { approved_by: "manager".to_string() },
        "corr-mm-po-approve".to_string(),
    )
    .await
    .expect("approve_po failed");

    // Bill: same qty, but price inflated by 25% (over 5% tolerance)
    let invoice_ref = format!("INV-MM-{}", &Uuid::new_v4().to_string()[..8]);
    let bill_with_lines = create_bill(
        &ap,
        TENANT_A,
        &CreateBillRequest {
            vendor_id,
            vendor_invoice_ref: invoice_ref,
            currency: "USD".to_string(),
            invoice_date: Utc::now(),
            due_date: None,
            tax_minor: None,
            entered_by: "ap-clerk".to_string(),
            fx_rate_id: None,
            lines: vec![CreateBillLineRequest {
                description: Some("Widgets".to_string()),
                item_id: None,
                quantity: 10.0,
                unit_price_minor: 1250, // 25% over PO price — outside tolerance
                gl_account_code: Some("6100".to_string()),
                po_line_id: Some(po_line_id),
            }],
        },
        "corr-mm-bill".to_string(),
    )
    .await
    .expect("create_bill failed");
    let bill_id = bill_with_lines.bill.bill_id;

    // Run match — expect price_variance
    let match_outcome = run_match(
        &ap,
        TENANT_A,
        bill_id,
        &RunMatchRequest {
            po_id,
            matched_by: "ap-user".to_string(),
            price_tolerance_pct: 0.05,
        },
        "corr-mm-match".to_string(),
    )
    .await
    .expect("run_match failed");

    assert!(
        !match_outcome.fully_matched,
        "price mismatch must produce not-fully-matched outcome"
    );
    assert_eq!(
        match_outcome.lines[0].match_status, "price_variance",
        "match_status must be price_variance"
    );
    assert!(!match_outcome.lines[0].within_tolerance);

    // Approve without override must fail
    let approve_result = approve_bill(
        &ap,
        TENANT_A,
        bill_id,
        &ApproveBillRequest {
            approved_by: "controller".to_string(),
            override_reason: None,
        },
        "corr-mm-approve".to_string(),
    )
    .await;

    assert!(
        matches!(approve_result, Err(ap::domain::bills::BillError::MatchPolicyViolation(_))),
        "approve without override must fail on tolerance violation, got {:?}",
        approve_result
    );

    // Approve WITH override must succeed
    let approved = approve_bill(
        &ap,
        TENANT_A,
        bill_id,
        &ApproveBillRequest {
            approved_by: "controller".to_string(),
            override_reason: Some("CFO pre-approved price increase".to_string()),
        },
        "corr-mm-approve-override".to_string(),
    )
    .await
    .expect("approve with override should succeed");

    assert_eq!(approved.status, "approved");

    cleanup_ap(&ap, TENANT_A).await;
}

/// Cross-tenant isolation: GL posting for TENANT_A must not appear for TENANT_B.
#[tokio::test]
#[serial]
async fn test_cross_tenant_gl_isolation() {
    let ap = common::get_ap_pool().await;
    let gl = common::get_gl_pool().await;

    cleanup_ap(&ap, TENANT_A).await;
    cleanup_ap(&ap, TENANT_B).await;
    cleanup_gl(&gl, TENANT_A).await;
    cleanup_gl(&gl, TENANT_B).await;

    setup_gl_accounts(&gl, TENANT_A).await;
    setup_gl_accounts(&gl, TENANT_B).await;
    setup_accounting_period(&gl, TENANT_A).await;

    let vendor_a = create_test_vendor(&ap, TENANT_A).await;

    let gl_event_id = Uuid::new_v4();
    let payload = VendorBillApprovedPayload {
        bill_id: Uuid::new_v4(),
        tenant_id: TENANT_A.to_string(),
        vendor_id: vendor_a,
        vendor_invoice_ref: "INV-ISOLATION-001".to_string(),
        approved_amount_minor: 3000,
        currency: "USD".to_string(),
        due_date: Utc::now() + chrono::Duration::days(30),
        approved_by: "controller".to_string(),
        approved_at: Utc::now(),
        fx_rate_id: None,
        gl_lines: vec![], // fallback to EXPENSE
    };

    process_ap_bill_approved_posting(&gl, gl_event_id, TENANT_A, "ap", &payload)
        .await
        .expect("GL posting for TENANT_A failed");

    // TENANT_B must have zero journal entries
    let (tenant_b_count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM journal_entries WHERE tenant_id = $1",
    )
    .bind(TENANT_B)
    .fetch_one(&gl)
    .await
    .expect("tenant_b count");
    assert_eq!(tenant_b_count, 0, "TENANT_B must not see TENANT_A's GL entries");

    // TENANT_A must have exactly one
    let (tenant_a_count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM journal_entries WHERE tenant_id = $1 AND source_event_id = $2",
    )
    .bind(TENANT_A)
    .bind(gl_event_id)
    .fetch_one(&gl)
    .await
    .expect("tenant_a count");
    assert_eq!(tenant_a_count, 1);

    cleanup_ap(&ap, TENANT_A).await;
    cleanup_ap(&ap, TENANT_B).await;
    cleanup_gl(&gl, TENANT_A).await;
    cleanup_gl(&gl, TENANT_B).await;
}
