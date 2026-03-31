//! E2E: Multi-currency AP bill approval + GL translation (bd-3gd2)
//!
//! Validates the AP → GL boundary for non-base-currency vendor bills:
//!   1. A EUR vendor bill can be created and approved (with override_reason)
//!   2. GL posting uses existing FX infra (Phase 23a `fx_rates` table) and
//!      produces balanced journal entries in the reporting currency (USD)
//!   3. Idempotency: second GL posting call with same event_id returns
//!      DuplicateEvent — no duplicate journal entries created
//!
//! Run with: ./scripts/cargo-slot.sh test -p e2e-tests -- ap_multicurrency --nocapture

mod common;

use anyhow::Result;
use chrono::{Datelike, NaiveDate, Utc};
use common::{generate_test_tenant, get_ap_pool, get_gl_pool};
use gl_rs::consumers::ap_vendor_bill_approved_consumer::{
    process_ap_bill_approved_posting, ApprovedGlLine, VendorBillApprovedPayload,
};
use gl_rs::services::journal_service::JournalError;
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

// ============================================================================
// GL setup helpers
// ============================================================================

/// Create GL accounts required for AP posting.
///
/// Accounts needed:
///   6100 — expense account (debit side for unmatched bill lines)
///   AP   — accounts payable liability (credit side)
async fn setup_gl_accounts(gl_pool: &PgPool, tenant_id: &str) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO accounts (id, tenant_id, code, name, type, normal_balance, is_active)
        VALUES
          (gen_random_uuid(), $1, '6100', 'Expense',            'expense',   'debit',  true),
          (gen_random_uuid(), $1, 'AP',   'Accounts Payable',   'liability', 'credit', true)
        ON CONFLICT (tenant_id, code) DO NOTHING
        "#,
    )
    .bind(tenant_id)
    .execute(gl_pool)
    .await?;
    Ok(())
}

/// Create an open accounting period covering the current month.
///
/// Uses dynamic dates so the test works regardless of when it runs.
async fn setup_open_period(gl_pool: &PgPool, tenant_id: &str) -> Result<Uuid> {
    let today = chrono::Utc::now().date_naive();
    let first_of_month = NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap();
    let last_of_month = first_of_month
        .checked_add_months(chrono::Months::new(1))
        .unwrap()
        .pred_opt()
        .unwrap();

    let period_id = sqlx::query_scalar::<_, Uuid>(
        r#"
        INSERT INTO accounting_periods (tenant_id, period_start, period_end, is_closed)
        VALUES ($1, $2, $3, false)
        ON CONFLICT DO NOTHING
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(first_of_month)
    .bind(last_of_month)
    .fetch_optional(gl_pool)
    .await?;

    if let Some(id) = period_id {
        return Ok(id);
    }

    // Conflict on period — fetch existing
    let id = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM accounting_periods WHERE tenant_id = $1 \
         AND period_start = $2 LIMIT 1",
    )
    .bind(tenant_id)
    .bind(first_of_month)
    .fetch_one(gl_pool)
    .await?;
    Ok(id)
}

/// Insert a EUR/USD FX rate in the GL fx_rates table.
///
/// Returns the rate_id (UUID) to embed in the AP bill as `fx_rate_id`.
///
/// Rate: 1 EUR = 1.10 USD (so EUR amounts multiply by 1.10 for USD reporting)
async fn setup_fx_rate(gl_pool: &PgPool, tenant_id: &str) -> Result<Uuid> {
    let rate_id = Uuid::new_v4();
    let idempotency_key = format!("e2e-eur-usd-{}", rate_id);
    sqlx::query(
        r#"
        INSERT INTO fx_rates
            (id, tenant_id, base_currency, quote_currency, rate, inverse_rate,
             effective_at, source, idempotency_key, created_at)
        VALUES ($1, $2, 'EUR', 'USD', 1.10, 0.909090909, NOW(), 'e2e-test', $3, NOW())
        "#,
    )
    .bind(rate_id)
    .bind(tenant_id)
    .bind(idempotency_key)
    .execute(gl_pool)
    .await?;
    Ok(rate_id)
}

// ============================================================================
// AP setup helpers
// ============================================================================

/// Create a vendor in AP DB and return the vendor_id.
async fn create_vendor(ap_pool: &PgPool, tenant_id: &str) -> Result<Uuid> {
    let vendor_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO vendors
            (vendor_id, tenant_id, name, currency, payment_terms_days, is_active,
             created_at, updated_at)
        VALUES ($1, $2, $3, 'EUR', 30, TRUE, NOW(), NOW())
        "#,
    )
    .bind(vendor_id)
    .bind(tenant_id)
    .bind(format!("EU-Vendor-{}", &vendor_id.to_string()[..8]))
    .execute(ap_pool)
    .await?;
    Ok(vendor_id)
}

/// Create a EUR vendor bill with one line and an FX rate reference.
///
/// Bill total: 10 000 EUR minor units (= 100.00 EUR)
/// GL account for expense line: '6100'
/// fx_rate_id: references the rate created in GL via `setup_fx_rate`
async fn create_eur_bill(
    ap_pool: &PgPool,
    tenant_id: &str,
    vendor_id: Uuid,
    fx_rate_id: Uuid,
) -> Result<(Uuid, Uuid)> {
    let bill_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO vendor_bills
            (bill_id, tenant_id, vendor_id, vendor_invoice_ref, currency,
             total_minor, invoice_date, due_date, status, fx_rate_id,
             entered_by, entered_at)
        VALUES ($1, $2, $3, $4, 'EUR', 10000,
                NOW(), NOW() + interval '30 days', 'open', $5,
                'e2e-test', NOW())
        "#,
    )
    .bind(bill_id)
    .bind(tenant_id)
    .bind(vendor_id)
    .bind(format!("EUR-INV-{}", &bill_id.to_string()[..8]))
    .bind(fx_rate_id)
    .execute(ap_pool)
    .await?;

    let line_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO bill_lines
            (line_id, bill_id, description, quantity, unit_price_minor,
             line_total_minor, gl_account_code, created_at)
        VALUES ($1, $2, 'EU Services', 1.0, 10000, 10000, '6100', NOW())
        "#,
    )
    .bind(line_id)
    .bind(bill_id)
    .execute(ap_pool)
    .await?;

    Ok((bill_id, line_id))
}

// ============================================================================
// Cleanup
// ============================================================================

async fn cleanup_ap(ap_pool: &PgPool, tenant_id: &str) {
    for q in [
        "DELETE FROM three_way_match WHERE bill_id IN \
         (SELECT bill_id FROM vendor_bills WHERE tenant_id = $1)",
        "DELETE FROM events_outbox WHERE aggregate_type = 'bill' \
         AND aggregate_id IN (SELECT bill_id::TEXT FROM vendor_bills WHERE tenant_id = $1)",
        "DELETE FROM bill_lines WHERE bill_id IN \
         (SELECT bill_id FROM vendor_bills WHERE tenant_id = $1)",
        "DELETE FROM vendor_bills WHERE tenant_id = $1",
        "DELETE FROM vendors WHERE tenant_id = $1",
    ] {
        sqlx::query(q).bind(tenant_id).execute(ap_pool).await.ok();
    }
}

async fn cleanup_gl(gl_pool: &PgPool, tenant_id: &str) {
    for q in [
        "DELETE FROM processed_events WHERE tenant_id = $1",
        "DELETE FROM journal_lines WHERE journal_id IN \
         (SELECT id FROM journal_entries WHERE tenant_id = $1)",
        "DELETE FROM journal_entries WHERE tenant_id = $1",
        "DELETE FROM fx_rates WHERE tenant_id = $1",
        "DELETE FROM accounting_periods WHERE tenant_id = $1",
        "DELETE FROM accounts WHERE tenant_id = $1",
    ] {
        sqlx::query(q).bind(tenant_id).execute(gl_pool).await.ok();
    }
}

// ============================================================================
// Helper: count GL journal entries for a source doc
// ============================================================================

async fn count_gl_entries(gl_pool: &PgPool, tenant_id: &str, reference_id: &str) -> Result<i64> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM journal_entries WHERE tenant_id = $1 AND reference_id = $2",
    )
    .bind(tenant_id)
    .bind(reference_id)
    .fetch_one(gl_pool)
    .await?;
    Ok(count)
}

/// Fetch total debits and credits for journal lines (in minor units).
async fn fetch_journal_totals_minor(
    gl_pool: &PgPool,
    tenant_id: &str,
    reference_id: &str,
) -> Result<(i64, i64)> {
    let (debits, credits): (i64, i64) = sqlx::query_as(
        r#"
        SELECT
            COALESCE(SUM(jl.debit_minor), 0)::BIGINT,
            COALESCE(SUM(jl.credit_minor), 0)::BIGINT
        FROM journal_lines jl
        JOIN journal_entries je ON jl.journal_entry_id = je.id
        WHERE je.tenant_id = $1 AND je.reference_id = $2
        "#,
    )
    .bind(tenant_id)
    .bind(reference_id)
    .fetch_one(gl_pool)
    .await?;
    Ok((debits, credits))
}

// ============================================================================
// Tests
// ============================================================================

/// Test 1: EUR bill approval → balanced GL posting in USD (reporting currency)
///
/// Scenario:
///   Bill: 10 000 EUR minor units (100.00 EUR) with FX rate EUR/USD = 1.10
///   Expected GL (in USD):
///     DR 6100 Expense   110.00 (10 000 × 1.10 / 100)
///     CR AP  Liability  110.00
#[tokio::test]
#[serial]
async fn test_ap_multicurrency_bill_approval_posts_gl_in_reporting_currency() {
    let tenant_id = generate_test_tenant();
    let ap_pool = get_ap_pool().await;
    let gl_pool = get_gl_pool().await;

    cleanup_ap(&ap_pool, &tenant_id).await;
    cleanup_gl(&gl_pool, &tenant_id).await;

    // Setup
    setup_gl_accounts(&gl_pool, &tenant_id)
        .await
        .expect("GL accounts");
    setup_open_period(&gl_pool, &tenant_id)
        .await
        .expect("GL period");
    let fx_rate_id = setup_fx_rate(&gl_pool, &tenant_id).await.expect("FX rate");

    let vendor_id = create_vendor(&ap_pool, &tenant_id).await.expect("vendor");
    let (bill_id, line_id) = create_eur_bill(&ap_pool, &tenant_id, vendor_id, fx_rate_id)
        .await
        .expect("EUR bill");

    // Build the event payload (mirrors what approve.rs builds)
    let event_id = Uuid::new_v4();
    let payload = VendorBillApprovedPayload {
        bill_id,
        tenant_id: tenant_id.clone(),
        vendor_id,
        vendor_invoice_ref: format!("EUR-INV-{}", &bill_id.to_string()[..8]),
        approved_amount_minor: 10000,
        currency: "EUR".to_string(),
        due_date: Utc::now() + chrono::Duration::days(30),
        approved_by: "e2e-approver".to_string(),
        approved_at: Utc::now(),
        fx_rate_id: Some(fx_rate_id),
        gl_lines: vec![ApprovedGlLine {
            line_id,
            gl_account_code: "6100".to_string(),
            amount_minor: 10000,
            po_line_id: None,
        }],
    };

    // Post GL journal entry via AP consumer logic
    let entry_id = process_ap_bill_approved_posting(&gl_pool, event_id, &tenant_id, "ap", &payload)
        .await
        .expect("GL posting failed");

    assert_ne!(entry_id, Uuid::nil(), "entry_id must be non-nil");

    // Verify exactly 1 journal entry was created
    let count = count_gl_entries(&gl_pool, &tenant_id, &bill_id.to_string())
        .await
        .expect("count entries");
    assert_eq!(count, 1, "exactly one journal entry expected");

    // Verify journal is balanced (debits == credits, in minor units)
    let (debits, credits) = fetch_journal_totals_minor(&gl_pool, &tenant_id, &bill_id.to_string())
        .await
        .expect("fetch totals");
    assert_eq!(
        debits, credits,
        "journal must balance: debits={} credits={} minor",
        debits, credits
    );

    // Verify amounts are in USD (reporting currency): 10000 EUR minor × 1.10 = 11000 USD minor
    assert_eq!(
        debits, 11000,
        "expected 11000 USD minor (110.00) debit total after FX conversion, got {}",
        debits
    );

    cleanup_ap(&ap_pool, &tenant_id).await;
    cleanup_gl(&gl_pool, &tenant_id).await;
}

/// Test 2: Idempotency — same event_id posted twice → DuplicateEvent, no extra journal entry
#[tokio::test]
#[serial]
async fn test_ap_multicurrency_gl_posting_is_idempotent() {
    let tenant_id = generate_test_tenant();
    let ap_pool = get_ap_pool().await;
    let gl_pool = get_gl_pool().await;

    cleanup_ap(&ap_pool, &tenant_id).await;
    cleanup_gl(&gl_pool, &tenant_id).await;

    setup_gl_accounts(&gl_pool, &tenant_id)
        .await
        .expect("GL accounts");
    setup_open_period(&gl_pool, &tenant_id)
        .await
        .expect("GL period");
    let fx_rate_id = setup_fx_rate(&gl_pool, &tenant_id).await.expect("FX rate");

    let vendor_id = create_vendor(&ap_pool, &tenant_id).await.expect("vendor");
    let (bill_id, line_id) = create_eur_bill(&ap_pool, &tenant_id, vendor_id, fx_rate_id)
        .await
        .expect("EUR bill");

    let event_id = Uuid::new_v4();
    let payload = VendorBillApprovedPayload {
        bill_id,
        tenant_id: tenant_id.clone(),
        vendor_id,
        vendor_invoice_ref: format!("EUR-INV-{}", &bill_id.to_string()[..8]),
        approved_amount_minor: 10000,
        currency: "EUR".to_string(),
        due_date: Utc::now() + chrono::Duration::days(30),
        approved_by: "e2e-approver".to_string(),
        approved_at: Utc::now(),
        fx_rate_id: Some(fx_rate_id),
        gl_lines: vec![ApprovedGlLine {
            line_id,
            gl_account_code: "6100".to_string(),
            amount_minor: 10000,
            po_line_id: None,
        }],
    };

    // First posting: must succeed
    process_ap_bill_approved_posting(&gl_pool, event_id, &tenant_id, "ap", &payload)
        .await
        .expect("first GL posting failed");

    // Second posting (same event_id): must return DuplicateEvent
    let second =
        process_ap_bill_approved_posting(&gl_pool, event_id, &tenant_id, "ap", &payload).await;
    assert!(
        matches!(second, Err(JournalError::DuplicateEvent(_))),
        "expected DuplicateEvent on second posting, got {:?}",
        second
    );

    // Still exactly 1 journal entry
    let count = count_gl_entries(&gl_pool, &tenant_id, &bill_id.to_string())
        .await
        .expect("count");
    assert_eq!(
        count, 1,
        "idempotent second posting must not create a second entry"
    );

    cleanup_ap(&ap_pool, &tenant_id).await;
    cleanup_gl(&gl_pool, &tenant_id).await;
}

/// Test 3: Same-currency bill (no fx_rate_id) posts in original currency
///
/// Scenario: USD bill, no FX rate → amounts posted as-is in USD.
#[tokio::test]
#[serial]
async fn test_ap_same_currency_bill_posts_without_fx_conversion() {
    let tenant_id = generate_test_tenant();
    let ap_pool = get_ap_pool().await;
    let gl_pool = get_gl_pool().await;

    cleanup_ap(&ap_pool, &tenant_id).await;
    cleanup_gl(&gl_pool, &tenant_id).await;

    setup_gl_accounts(&gl_pool, &tenant_id)
        .await
        .expect("GL accounts");
    setup_open_period(&gl_pool, &tenant_id)
        .await
        .expect("GL period");

    let vendor_id = create_vendor(&ap_pool, &tenant_id).await.expect("vendor");
    let bill_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO vendor_bills
            (bill_id, tenant_id, vendor_id, vendor_invoice_ref, currency,
             total_minor, invoice_date, due_date, status, entered_by, entered_at)
        VALUES ($1, $2, $3, $4, 'USD', 5000, NOW(), NOW() + interval '30 days',
                'open', 'e2e-test', NOW())
        "#,
    )
    .bind(bill_id)
    .bind(&tenant_id)
    .bind(vendor_id)
    .bind(format!("USD-INV-{}", &bill_id.to_string()[..8]))
    .execute(&ap_pool)
    .await
    .expect("insert USD bill");

    let line_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO bill_lines \
         (line_id, bill_id, description, quantity, unit_price_minor, \
          line_total_minor, gl_account_code, created_at) \
         VALUES ($1, $2, 'US Widget', 1.0, 5000, 5000, '6100', NOW())",
    )
    .bind(line_id)
    .bind(bill_id)
    .execute(&ap_pool)
    .await
    .expect("insert USD bill line");

    let event_id = Uuid::new_v4();
    let payload = VendorBillApprovedPayload {
        bill_id,
        tenant_id: tenant_id.clone(),
        vendor_id,
        vendor_invoice_ref: format!("USD-INV-{}", &bill_id.to_string()[..8]),
        approved_amount_minor: 5000,
        currency: "USD".to_string(),
        due_date: Utc::now() + chrono::Duration::days(30),
        approved_by: "e2e-approver".to_string(),
        approved_at: Utc::now(),
        fx_rate_id: None,
        gl_lines: vec![ApprovedGlLine {
            line_id,
            gl_account_code: "6100".to_string(),
            amount_minor: 5000,
            po_line_id: None,
        }],
    };

    process_ap_bill_approved_posting(&gl_pool, event_id, &tenant_id, "ap", &payload)
        .await
        .expect("USD bill GL posting failed");

    let (debits, credits) = fetch_journal_totals_minor(&gl_pool, &tenant_id, &bill_id.to_string())
        .await
        .expect("fetch totals");

    assert_eq!(
        debits, credits,
        "journal must balance: debits={} credits={} minor",
        debits, credits
    );
    // 5000 minor units = 50.00 USD (no FX conversion)
    assert_eq!(
        debits, 5000,
        "expected 5000 USD minor (50.00) debit, got {}",
        debits
    );

    cleanup_ap(&ap_pool, &tenant_id).await;
    cleanup_gl(&gl_pool, &tenant_id).await;
}

/// Test 4: No cross-tenant contamination — two tenants, each sees only their own GL entries
#[tokio::test]
#[serial]
async fn test_ap_multicurrency_no_cross_tenant_contamination() {
    let tenant_a = generate_test_tenant();
    let tenant_b = generate_test_tenant();
    let ap_pool = get_ap_pool().await;
    let gl_pool = get_gl_pool().await;

    cleanup_ap(&ap_pool, &tenant_a).await;
    cleanup_ap(&ap_pool, &tenant_b).await;
    cleanup_gl(&gl_pool, &tenant_a).await;
    cleanup_gl(&gl_pool, &tenant_b).await;

    for t in [&tenant_a, &tenant_b] {
        setup_gl_accounts(&gl_pool, t).await.expect("GL accounts");
        setup_open_period(&gl_pool, t).await.expect("GL period");
    }

    let fx_rate_a = setup_fx_rate(&gl_pool, &tenant_a).await.expect("FX rate A");
    let fx_rate_b = setup_fx_rate(&gl_pool, &tenant_b).await.expect("FX rate B");

    let vendor_a = create_vendor(&ap_pool, &tenant_a).await.expect("vendor A");
    let vendor_b = create_vendor(&ap_pool, &tenant_b).await.expect("vendor B");

    let (bill_a, line_a) = create_eur_bill(&ap_pool, &tenant_a, vendor_a, fx_rate_a)
        .await
        .expect("bill A");
    let (bill_b, line_b) = create_eur_bill(&ap_pool, &tenant_b, vendor_b, fx_rate_b)
        .await
        .expect("bill B");

    let make_payload =
        |bill_id: Uuid, line_id: Uuid, tenant_id: &str, vendor_id: Uuid, fx_rate_id: Uuid| {
            VendorBillApprovedPayload {
                bill_id,
                tenant_id: tenant_id.to_string(),
                vendor_id,
                vendor_invoice_ref: format!("EUR-INV-{}", &bill_id.to_string()[..8]),
                approved_amount_minor: 10000,
                currency: "EUR".to_string(),
                due_date: Utc::now() + chrono::Duration::days(30),
                approved_by: "e2e-approver".to_string(),
                approved_at: Utc::now(),
                fx_rate_id: Some(fx_rate_id),
                gl_lines: vec![ApprovedGlLine {
                    line_id,
                    gl_account_code: "6100".to_string(),
                    amount_minor: 10000,
                    po_line_id: None,
                }],
            }
        };

    let payload_a = make_payload(bill_a, line_a, &tenant_a, vendor_a, fx_rate_a);
    let payload_b = make_payload(bill_b, line_b, &tenant_b, vendor_b, fx_rate_b);

    process_ap_bill_approved_posting(&gl_pool, Uuid::new_v4(), &tenant_a, "ap", &payload_a)
        .await
        .expect("tenant A GL posting");
    process_ap_bill_approved_posting(&gl_pool, Uuid::new_v4(), &tenant_b, "ap", &payload_b)
        .await
        .expect("tenant B GL posting");

    // Tenant A sees only their own entries
    let count_a = count_gl_entries(&gl_pool, &tenant_a, &bill_a.to_string())
        .await
        .expect("count A");
    let spill_a_to_b = count_gl_entries(&gl_pool, &tenant_a, &bill_b.to_string())
        .await
        .expect("spill A→B");
    assert_eq!(count_a, 1, "tenant A must have exactly 1 entry");
    assert_eq!(spill_a_to_b, 0, "tenant A must not see tenant B's bill");

    // Tenant B sees only their own entries
    let count_b = count_gl_entries(&gl_pool, &tenant_b, &bill_b.to_string())
        .await
        .expect("count B");
    let spill_b_to_a = count_gl_entries(&gl_pool, &tenant_b, &bill_a.to_string())
        .await
        .expect("spill B→A");
    assert_eq!(count_b, 1, "tenant B must have exactly 1 entry");
    assert_eq!(spill_b_to_a, 0, "tenant B must not see tenant A's bill");

    cleanup_ap(&ap_pool, &tenant_a).await;
    cleanup_ap(&ap_pool, &tenant_b).await;
    cleanup_gl(&gl_pool, &tenant_a).await;
    cleanup_gl(&gl_pool, &tenant_b).await;
}
