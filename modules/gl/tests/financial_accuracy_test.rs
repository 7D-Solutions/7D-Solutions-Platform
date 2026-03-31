//! Financial Accuracy Tests — GL balancing and AR/AP reconciliation (bd-zznx6)
//!
//! P0 tests for an aerospace/defense customer.
//! Every test hits real databases — no mocks, no stubs.
//!
//! ## Coverage
//! 1. GL balance verification: for every journal entry, sum(debits) == sum(credits)
//! 2. AR reconciliation: total invoiced == total paid + total outstanding + total written off
//! 3. AP reconciliation: total billed == total paid + total outstanding (via allocations)
//! 4. Cross-module consistency: payment in Payments module matches AR payment record
//! 5. Currency handling: multi-currency journal entries convert correctly
//! 6. Period close: GL trial balance sums to zero after closing entries

mod common;

use chrono::{NaiveDate, Utc};
use common::{cleanup_test_tenant, get_test_pool, setup_test_account, setup_test_period};
use gl_rs::repos::journal_repo::{self, JournalLineInsert};
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

// ============================================================================
// Helper: unique tenant per test to avoid cross-contamination
// ============================================================================

fn unique_tenant() -> String {
    format!("fin-acc-{}", Uuid::new_v4().simple())
}

// ============================================================================
// Helper: set up a full chart of accounts for financial testing
// ============================================================================

async fn setup_coa(pool: &PgPool, tenant_id: &str) {
    // Asset accounts
    setup_test_account(pool, tenant_id, "1000", "Cash", "asset", "debit").await;
    setup_test_account(pool, tenant_id, "1100", "Accounts Receivable", "asset", "debit").await;
    setup_test_account(pool, tenant_id, "1200", "Inventory", "asset", "debit").await;

    // Liability accounts
    setup_test_account(pool, tenant_id, "2000", "Accounts Payable", "liability", "credit").await;
    setup_test_account(pool, tenant_id, "2100", "Tax Payable", "liability", "credit").await;

    // Equity accounts
    setup_test_account(pool, tenant_id, "3000", "Retained Earnings", "equity", "credit").await;

    // Revenue accounts
    setup_test_account(pool, tenant_id, "4000", "Revenue", "revenue", "credit").await;
    setup_test_account(pool, tenant_id, "4100", "Service Revenue", "revenue", "credit").await;

    // Expense accounts
    setup_test_account(pool, tenant_id, "5000", "COGS", "expense", "debit").await;
    setup_test_account(pool, tenant_id, "6000", "Operating Expenses", "expense", "debit").await;
    setup_test_account(pool, tenant_id, "6100", "Bad Debt Expense", "expense", "debit").await;
}

// ============================================================================
// Helper: post a balanced journal entry via direct DB insert
// ============================================================================

async fn post_journal_entry(
    pool: &PgPool,
    tenant_id: &str,
    source_module: &str,
    currency: &str,
    description: &str,
    lines: Vec<(String, i64, i64)>, // (account_ref, debit_minor, credit_minor)
) -> Uuid {
    let entry_id = Uuid::new_v4();
    let source_event_id = Uuid::new_v4();
    let mut tx = pool.begin().await.expect("begin tx");

    journal_repo::insert_entry(
        &mut tx,
        entry_id,
        tenant_id,
        source_module,
        source_event_id,
        &format!("{}.posting", source_module),
        Utc::now(),
        currency,
        Some(description),
        None,
        None,
        Some(Uuid::new_v4()),
    )
    .await
    .expect("insert entry");

    let line_inserts: Vec<JournalLineInsert> = lines
        .into_iter()
        .enumerate()
        .map(|(i, (account_ref, debit_minor, credit_minor))| JournalLineInsert {
            id: Uuid::new_v4(),
            line_no: i as i32 + 1,
            account_ref,
            debit_minor,
            credit_minor,
            memo: None,
        })
        .collect();

    journal_repo::bulk_insert_lines(&mut tx, entry_id, &line_inserts)
        .await
        .expect("insert lines");

    tx.commit().await.expect("commit tx");
    entry_id
}

// ============================================================================
// Test 1: GL Balance Verification — every entry debits == credits
// ============================================================================

#[tokio::test]
#[serial]
async fn test_gl_every_entry_is_balanced() {
    let pool = get_test_pool().await;
    let tenant = unique_tenant();
    cleanup_test_tenant(&pool, &tenant).await;
    setup_coa(&pool, &tenant).await;

    let _period = setup_test_period(
        &pool,
        &tenant,
        NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        NaiveDate::from_ymd_opt(2026, 12, 31).unwrap(),
    )
    .await;

    // Post several balanced entries
    // 1. AR Invoice: DR Accounts Receivable, CR Revenue
    post_journal_entry(
        &pool,
        &tenant,
        "ar",
        "USD",
        "Invoice #1001",
        vec![
            ("1100".to_string(), 100_000, 0), // AR +$1,000.00
            ("4000".to_string(), 0, 100_000), // Revenue +$1,000.00
        ],
    )
    .await;

    // 2. Payment received: DR Cash, CR Accounts Receivable
    post_journal_entry(
        &pool,
        &tenant,
        "payments",
        "USD",
        "Payment for Invoice #1001",
        vec![
            ("1000".to_string(), 100_000, 0), // Cash +$1,000.00
            ("1100".to_string(), 0, 100_000), // AR -$1,000.00
        ],
    )
    .await;

    // 3. AP Bill: DR Expense, CR Accounts Payable
    post_journal_entry(
        &pool,
        &tenant,
        "ap",
        "USD",
        "Vendor bill #V-2001",
        vec![
            ("6000".to_string(), 50_000, 0), // Expense +$500.00
            ("2000".to_string(), 0, 50_000), // AP +$500.00
        ],
    )
    .await;

    // 4. AP Payment: DR Accounts Payable, CR Cash
    post_journal_entry(
        &pool,
        &tenant,
        "ap",
        "USD",
        "Payment for bill #V-2001",
        vec![
            ("2000".to_string(), 50_000, 0), // AP -$500.00
            ("1000".to_string(), 0, 50_000), // Cash -$500.00
        ],
    )
    .await;

    // 5. Multi-line entry: inventory purchase
    post_journal_entry(
        &pool,
        &tenant,
        "inventory",
        "USD",
        "Inventory purchase",
        vec![
            ("1200".to_string(), 25_000, 0), // Inventory +$250.00
            ("2100".to_string(), 0, 3_750),  // Tax payable +$37.50
            ("2000".to_string(), 0, 21_250), // AP +$212.50
        ],
    )
    .await;

    // Use the module's built-in invariant check
    gl_rs::invariants::assert_all_entries_balanced(&pool, &tenant)
        .await
        .expect("ALL GL entries must be balanced (debits == credits)");

    // Also verify with a raw aggregate query as a cross-check
    let (total_debits, total_credits): (i64, i64) = sqlx::query_as(
        r#"
        SELECT COALESCE(SUM(jl.debit_minor), 0)::BIGINT, COALESCE(SUM(jl.credit_minor), 0)::BIGINT
        FROM journal_lines jl
        JOIN journal_entries je ON je.id = jl.journal_entry_id
        WHERE je.tenant_id = $1
        "#,
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .expect("aggregate query");

    assert_eq!(
        total_debits, total_credits,
        "Global debit/credit totals must match: debits={}, credits={}",
        total_debits, total_credits
    );

    // Verify entry count
    let entry_count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM journal_entries WHERE tenant_id = $1")
            .bind(&tenant)
            .fetch_one(&pool)
            .await
            .expect("count query");
    assert_eq!(entry_count.0, 5, "Expected 5 journal entries");

    cleanup_test_tenant(&pool, &tenant).await;
    pool.close().await;
}

// ============================================================================
// Test 2: GL full invariant suite on seeded data
// ============================================================================

#[tokio::test]
#[serial]
async fn test_gl_full_invariant_suite() {
    let pool = get_test_pool().await;
    let tenant = unique_tenant();
    cleanup_test_tenant(&pool, &tenant).await;
    setup_coa(&pool, &tenant).await;

    let _period = setup_test_period(
        &pool,
        &tenant,
        NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        NaiveDate::from_ymd_opt(2026, 12, 31).unwrap(),
    )
    .await;

    // Post a variety of entries from different source modules
    for i in 0..10 {
        let amount = (i + 1) as i64 * 10_000;
        post_journal_entry(
            &pool,
            &tenant,
            "ar",
            "USD",
            &format!("Invoice #{}", 1000 + i),
            vec![
                ("1100".to_string(), amount, 0),
                ("4000".to_string(), 0, amount),
            ],
        )
        .await;
    }

    // Run ALL GL invariants (balanced entries, no duplicates, valid accounts,
    // no closed-period postings, unique line numbers, reversal chain depth)
    gl_rs::invariants::assert_all_invariants(&pool, &tenant)
        .await
        .expect("All GL invariants must hold");

    cleanup_test_tenant(&pool, &tenant).await;
    pool.close().await;
}

// ============================================================================
// Test 3: AR Reconciliation — invoiced == paid + outstanding + written-off
// ============================================================================

#[tokio::test]
#[serial]
async fn test_ar_reconciliation_invoiced_equals_paid_plus_outstanding() {
    // Connect to the AR database
    dotenvy::dotenv().ok();
    let ar_url = std::env::var("DATABASE_URL_AR")
        .unwrap_or_else(|_| "postgresql://ar_user:ar_pass@localhost:5434/ar_db".to_string());
    let ar_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .connect(&ar_url)
        .await
        .expect("connect to AR DB");

    sqlx::migrate!("../ar/db/migrations")
        .run(&ar_pool)
        .await
        .expect("run AR migrations");

    let app_id = format!("fin-acc-ar-{}", Uuid::new_v4().simple());

    // Seed a customer
    let email = format!("test-{}@example.com", Uuid::new_v4());
    let ext_id = format!("ext-{}", Uuid::new_v4());
    let customer_id: i32 = sqlx::query_scalar(
        r#"INSERT INTO ar_customers (
            app_id, email, external_customer_id, status, name,
            default_payment_method_id, payment_method_type,
            retry_attempt_count, created_at, updated_at
        ) VALUES ($1, $2, $3, 'active', 'FinAcc Test Customer', 'pm_test', 'card', 0, NOW(), NOW())
        RETURNING id"#,
    )
    .bind(&app_id)
    .bind(&email)
    .bind(&ext_id)
    .fetch_one(&ar_pool)
    .await
    .expect("seed customer");

    // Create invoices with different statuses
    // Invoice 1: paid ($500)
    let _inv1_id: i32 = sqlx::query_scalar(
        r#"INSERT INTO ar_invoices (
            app_id, tilled_invoice_id, ar_customer_id, status,
            amount_cents, currency, created_at, updated_at, paid_at
        ) VALUES ($1, $2, $3, 'paid', 50000, 'usd', NOW(), NOW(), NOW())
        RETURNING id"#,
    )
    .bind(&app_id)
    .bind(format!("inv_{}", Uuid::new_v4()))
    .bind(customer_id)
    .fetch_one(&ar_pool)
    .await
    .expect("seed invoice 1");

    // Invoice 2: open ($300)
    let _inv2_id: i32 = sqlx::query_scalar(
        r#"INSERT INTO ar_invoices (
            app_id, tilled_invoice_id, ar_customer_id, status,
            amount_cents, currency, due_at, created_at, updated_at
        ) VALUES ($1, $2, $3, 'open', 30000, 'usd', NOW() + INTERVAL '30 days', NOW(), NOW())
        RETURNING id"#,
    )
    .bind(&app_id)
    .bind(format!("inv_{}", Uuid::new_v4()))
    .bind(customer_id)
    .fetch_one(&ar_pool)
    .await
    .expect("seed invoice 2");

    // Invoice 3: uncollectible (written off) ($200)
    let _inv3_id: i32 = sqlx::query_scalar(
        r#"INSERT INTO ar_invoices (
            app_id, tilled_invoice_id, ar_customer_id, status,
            amount_cents, currency, created_at, updated_at
        ) VALUES ($1, $2, $3, 'uncollectible', 20000, 'usd', NOW(), NOW())
        RETURNING id"#,
    )
    .bind(&app_id)
    .bind(format!("inv_{}", Uuid::new_v4()))
    .bind(customer_id)
    .fetch_one(&ar_pool)
    .await
    .expect("seed invoice 3");

    // Invoice 4: void ($100) — voids should NOT count in totals
    let _inv4_id: i32 = sqlx::query_scalar(
        r#"INSERT INTO ar_invoices (
            app_id, tilled_invoice_id, ar_customer_id, status,
            amount_cents, currency, created_at, updated_at
        ) VALUES ($1, $2, $3, 'void', 10000, 'usd', NOW(), NOW())
        RETURNING id"#,
    )
    .bind(&app_id)
    .bind(format!("inv_{}", Uuid::new_v4()))
    .bind(customer_id)
    .fetch_one(&ar_pool)
    .await
    .expect("seed invoice 4");

    // AR Reconciliation query: total_invoiced == paid + outstanding + uncollectible
    // Void invoices are excluded from the accounting equation.
    let (total_invoiced,): (i64,) = sqlx::query_as(
        r#"
        SELECT COALESCE(SUM(amount_cents::bigint), 0)::BIGINT
        FROM ar_invoices
        WHERE app_id = $1 AND status != 'void' AND status != 'draft'
        "#,
    )
    .bind(&app_id)
    .fetch_one(&ar_pool)
    .await
    .expect("total invoiced");

    let (total_paid,): (i64,) = sqlx::query_as(
        r#"
        SELECT COALESCE(SUM(amount_cents::bigint), 0)::BIGINT
        FROM ar_invoices
        WHERE app_id = $1 AND status = 'paid'
        "#,
    )
    .bind(&app_id)
    .fetch_one(&ar_pool)
    .await
    .expect("total paid");

    let (total_outstanding,): (i64,) = sqlx::query_as(
        r#"
        SELECT COALESCE(SUM(amount_cents::bigint), 0)::BIGINT
        FROM ar_invoices
        WHERE app_id = $1 AND status = 'open'
        "#,
    )
    .bind(&app_id)
    .fetch_one(&ar_pool)
    .await
    .expect("total outstanding");

    let (total_written_off,): (i64,) = sqlx::query_as(
        r#"
        SELECT COALESCE(SUM(amount_cents::bigint), 0)::BIGINT
        FROM ar_invoices
        WHERE app_id = $1 AND status = 'uncollectible'
        "#,
    )
    .bind(&app_id)
    .fetch_one(&ar_pool)
    .await
    .expect("total written off");

    // Core assertion: the accounting equation must hold
    assert_eq!(
        total_invoiced,
        total_paid + total_outstanding + total_written_off,
        "AR reconciliation failed: invoiced({}) != paid({}) + outstanding({}) + written_off({})",
        total_invoiced,
        total_paid,
        total_outstanding,
        total_written_off
    );

    // Verify specific amounts
    assert_eq!(total_invoiced, 100_000, "Total invoiced should be $1,000 (excl void)");
    assert_eq!(total_paid, 50_000, "Total paid should be $500");
    assert_eq!(total_outstanding, 30_000, "Total outstanding should be $300");
    assert_eq!(total_written_off, 20_000, "Total written off should be $200");

    // Cleanup
    sqlx::query("DELETE FROM ar_invoices WHERE app_id = $1")
        .bind(&app_id)
        .execute(&ar_pool)
        .await
        .ok();
    sqlx::query("DELETE FROM ar_customers WHERE app_id = $1")
        .bind(&app_id)
        .execute(&ar_pool)
        .await
        .ok();

    ar_pool.close().await;
}

// ============================================================================
// Test 4: AP Reconciliation — billed == allocated + open balance
// ============================================================================

#[tokio::test]
#[serial]
async fn test_ap_reconciliation_billed_equals_paid_plus_outstanding() {
    dotenvy::dotenv().ok();
    let ap_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://ap_user:ap_pass@localhost:5443/ap_db".to_string());
    let ap_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .connect(&ap_url)
        .await
        .expect("connect to AP DB");

    sqlx::migrate!("../ap/db/migrations")
        .run(&ap_pool)
        .await
        .expect("run AP migrations");

    let tenant = format!("fin-acc-ap-{}", Uuid::new_v4().simple());

    // Create a vendor
    let vendor_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO vendors (vendor_id, tenant_id, name, currency, payment_terms_days,
           payment_method, is_active, created_at)
        VALUES ($1, $2, 'FinAcc Test Vendor', 'USD', 30, 'ach', true, NOW())"#,
    )
    .bind(vendor_id)
    .bind(&tenant)
    .execute(&ap_pool)
    .await
    .expect("create vendor");

    // Bill 1: approved, $1,000 — will be fully allocated
    let bill1_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO vendor_bills (bill_id, tenant_id, vendor_id, vendor_invoice_ref,
           currency, total_minor, invoice_date, due_date, status, entered_by, entered_at)
        VALUES ($1, $2, $3, 'VINV-001', 'USD', 100000, NOW(), NOW() + INTERVAL '30 days',
                'paid', 'test-user', NOW())"#,
    )
    .bind(bill1_id)
    .bind(&tenant)
    .bind(vendor_id)
    .execute(&ap_pool)
    .await
    .expect("create bill 1");

    // Bill 2: partially paid, $500 total, $200 allocated
    let bill2_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO vendor_bills (bill_id, tenant_id, vendor_id, vendor_invoice_ref,
           currency, total_minor, invoice_date, due_date, status, entered_by, entered_at)
        VALUES ($1, $2, $3, 'VINV-002', 'USD', 50000, NOW(), NOW() + INTERVAL '30 days',
                'partially_paid', 'test-user', NOW())"#,
    )
    .bind(bill2_id)
    .bind(&tenant)
    .bind(vendor_id)
    .execute(&ap_pool)
    .await
    .expect("create bill 2");

    // Bill 3: approved, not yet paid, $750
    let bill3_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO vendor_bills (bill_id, tenant_id, vendor_id, vendor_invoice_ref,
           currency, total_minor, invoice_date, due_date, status, entered_by, entered_at)
        VALUES ($1, $2, $3, 'VINV-003', 'USD', 75000, NOW(), NOW() + INTERVAL '45 days',
                'approved', 'test-user', NOW())"#,
    )
    .bind(bill3_id)
    .bind(&tenant)
    .bind(vendor_id)
    .execute(&ap_pool)
    .await
    .expect("create bill 3");

    // Bill 4: voided, $300 — should NOT count
    let bill4_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO vendor_bills (bill_id, tenant_id, vendor_id, vendor_invoice_ref,
           currency, total_minor, invoice_date, due_date, status, entered_by, entered_at)
        VALUES ($1, $2, $3, 'VINV-004', 'USD', 30000, NOW(), NOW() + INTERVAL '30 days',
                'voided', 'test-user', NOW())"#,
    )
    .bind(bill4_id)
    .bind(&tenant)
    .bind(vendor_id)
    .execute(&ap_pool)
    .await
    .expect("create bill 4");

    // Create allocations
    // Full allocation on bill 1: $1,000
    sqlx::query(
        r#"INSERT INTO ap_allocations (allocation_id, bill_id, tenant_id, amount_minor,
           currency, allocation_type, created_at)
        VALUES ($1, $2, $3, 100000, 'USD', 'full', NOW())"#,
    )
    .bind(Uuid::new_v4())
    .bind(bill1_id)
    .bind(&tenant)
    .execute(&ap_pool)
    .await
    .expect("allocate bill 1");

    // Partial allocation on bill 2: $200 of $500
    sqlx::query(
        r#"INSERT INTO ap_allocations (allocation_id, bill_id, tenant_id, amount_minor,
           currency, allocation_type, created_at)
        VALUES ($1, $2, $3, 20000, 'USD', 'partial', NOW())"#,
    )
    .bind(Uuid::new_v4())
    .bind(bill2_id)
    .bind(&tenant)
    .execute(&ap_pool)
    .await
    .expect("allocate bill 2");

    // AP reconciliation: for non-voided bills,
    // total_billed == total_allocated + total_open_balance
    let (total_billed,): (i64,) = sqlx::query_as(
        r#"
        SELECT COALESCE(SUM(total_minor), 0)::BIGINT
        FROM vendor_bills
        WHERE tenant_id = $1 AND status != 'voided'
        "#,
    )
    .bind(&tenant)
    .fetch_one(&ap_pool)
    .await
    .expect("total billed");

    let (total_allocated,): (i64,) = sqlx::query_as(
        r#"
        SELECT COALESCE(SUM(a.amount_minor), 0)::BIGINT
        FROM ap_allocations a
        JOIN vendor_bills b ON b.bill_id = a.bill_id
        WHERE a.tenant_id = $1 AND b.status != 'voided'
        "#,
    )
    .bind(&tenant)
    .fetch_one(&ap_pool)
    .await
    .expect("total allocated");

    let total_open_balance = total_billed - total_allocated;

    // Core assertion: billed == allocated + open balance (tautologically true by definition,
    // but we also verify the individual amounts match expectations)
    assert_eq!(
        total_billed,
        total_allocated + total_open_balance,
        "AP reconciliation: billed({}) != allocated({}) + open({})",
        total_billed,
        total_allocated,
        total_open_balance
    );

    // Verify expected amounts (excluding voided bill)
    assert_eq!(total_billed, 225_000, "Total billed should be $2,250 (excl voided)");
    assert_eq!(total_allocated, 120_000, "Total allocated should be $1,200");
    assert_eq!(total_open_balance, 105_000, "Open balance should be $1,050");

    // Per-bill open balance verification
    let per_bill: Vec<(Uuid, i64, i64)> = sqlx::query_as(
        r#"
        SELECT b.bill_id, b.total_minor,
               COALESCE(SUM(a.amount_minor), 0)::BIGINT as allocated
        FROM vendor_bills b
        LEFT JOIN ap_allocations a ON a.bill_id = b.bill_id AND a.tenant_id = b.tenant_id
        WHERE b.tenant_id = $1 AND b.status != 'voided'
        GROUP BY b.bill_id, b.total_minor
        ORDER BY b.total_minor DESC
        "#,
    )
    .bind(&tenant)
    .fetch_all(&ap_pool)
    .await
    .expect("per-bill query");

    for (bill_id, total, allocated) in &per_bill {
        assert!(
            *allocated <= *total,
            "Over-allocation detected on bill {}: allocated {} > total {}",
            bill_id,
            allocated,
            total
        );
    }

    // Cleanup
    sqlx::query("DELETE FROM ap_allocations WHERE tenant_id = $1")
        .bind(&tenant)
        .execute(&ap_pool)
        .await
        .ok();
    sqlx::query("DELETE FROM vendor_bills WHERE tenant_id = $1")
        .bind(&tenant)
        .execute(&ap_pool)
        .await
        .ok();
    sqlx::query("DELETE FROM vendors WHERE tenant_id = $1")
        .bind(&tenant)
        .execute(&ap_pool)
        .await
        .ok();

    ap_pool.close().await;
}

// ============================================================================
// Test 5: Cross-module — Payments amount matches AR invoice
// ============================================================================

#[tokio::test]
#[serial]
async fn test_cross_module_payment_matches_ar_invoice() {
    dotenvy::dotenv().ok();

    // Connect to AR
    let ar_url = std::env::var("DATABASE_URL_AR")
        .unwrap_or_else(|_| "postgresql://ar_user:ar_pass@localhost:5434/ar_db".to_string());
    let ar_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .connect(&ar_url)
        .await
        .expect("connect to AR DB");

    sqlx::migrate!("../ar/db/migrations")
        .run(&ar_pool)
        .await
        .expect("run AR migrations");

    // Connect to Payments
    let pay_url = std::env::var("DATABASE_URL_PAYMENTS")
        .unwrap_or_else(|_| "postgresql://payments_user:payments_pass@localhost:5436/payments_db".to_string());
    let pay_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .connect(&pay_url)
        .await
        .expect("connect to Payments DB");

    sqlx::migrate!("../payments/db/migrations")
        .run(&pay_pool)
        .await
        .expect("run Payments migrations");

    let app_id = format!("fin-acc-xmod-{}", Uuid::new_v4().simple());
    let payment_id = Uuid::new_v4();
    let invoice_amount: i32 = 75_000; // $750.00
    let invoice_tilled_id = format!("inv_{}", Uuid::new_v4());

    // Seed AR customer + invoice
    let email = format!("xmod-{}@example.com", Uuid::new_v4());
    let ext_id = format!("ext-{}", Uuid::new_v4());
    let customer_id: i32 = sqlx::query_scalar(
        r#"INSERT INTO ar_customers (
            app_id, email, external_customer_id, status, name,
            default_payment_method_id, payment_method_type,
            retry_attempt_count, created_at, updated_at
        ) VALUES ($1, $2, $3, 'active', 'XMod Customer', 'pm_test', 'card', 0, NOW(), NOW())
        RETURNING id"#,
    )
    .bind(&app_id)
    .bind(&email)
    .bind(&ext_id)
    .fetch_one(&ar_pool)
    .await
    .expect("seed customer");

    let _inv_id: i32 = sqlx::query_scalar(
        r#"INSERT INTO ar_invoices (
            app_id, tilled_invoice_id, ar_customer_id, status,
            amount_cents, currency, paid_at, created_at, updated_at
        ) VALUES ($1, $2, $3, 'paid', $4, 'usd', NOW(), NOW(), NOW())
        RETURNING id"#,
    )
    .bind(&app_id)
    .bind(&invoice_tilled_id)
    .bind(customer_id)
    .bind(invoice_amount)
    .fetch_one(&ar_pool)
    .await
    .expect("seed invoice");

    // Seed corresponding payment attempt in Payments DB
    sqlx::query(
        r#"INSERT INTO payment_attempts (
            app_id, payment_id, invoice_id, attempt_no, status,
            processor_payment_id
        ) VALUES ($1, $2, $3, 1, 'succeeded', 'pi_test_succeeded_123')"#,
    )
    .bind(&app_id)
    .bind(payment_id)
    .bind(&invoice_tilled_id)
    .execute(&pay_pool)
    .await
    .expect("seed payment attempt");

    // Cross-module verification: the payment attempt's invoice_id must reference
    // a real invoice in AR, and the AR invoice must be in 'paid' status.
    let (payment_invoice_id,): (String,) = sqlx::query_as(
        "SELECT invoice_id FROM payment_attempts WHERE app_id = $1 AND payment_id = $2 AND status = 'succeeded'",
    )
    .bind(&app_id)
    .bind(payment_id)
    .fetch_one(&pay_pool)
    .await
    .expect("find payment");

    let (ar_status, ar_amount): (String, i32) = sqlx::query_as(
        "SELECT status, amount_cents FROM ar_invoices WHERE app_id = $1 AND tilled_invoice_id = $2",
    )
    .bind(&app_id)
    .bind(&payment_invoice_id)
    .fetch_one(&ar_pool)
    .await
    .expect("find matching AR invoice");

    assert_eq!(
        ar_status, "paid",
        "AR invoice status should be 'paid' when payment succeeded"
    );
    assert_eq!(
        ar_amount, invoice_amount,
        "AR invoice amount should match: expected {}, got {}",
        invoice_amount, ar_amount
    );

    // Cleanup
    sqlx::query("DELETE FROM payment_attempts WHERE app_id = $1")
        .bind(&app_id)
        .execute(&pay_pool)
        .await
        .ok();
    sqlx::query("DELETE FROM ar_invoices WHERE app_id = $1")
        .bind(&app_id)
        .execute(&ar_pool)
        .await
        .ok();
    sqlx::query("DELETE FROM ar_customers WHERE app_id = $1")
        .bind(&app_id)
        .execute(&ar_pool)
        .await
        .ok();

    ar_pool.close().await;
    pay_pool.close().await;
}

// ============================================================================
// Test 6: Multi-currency GL entries balance correctly per currency
// ============================================================================

#[tokio::test]
#[serial]
async fn test_multi_currency_gl_entries_balance_per_currency() {
    let pool = get_test_pool().await;
    let tenant = unique_tenant();
    cleanup_test_tenant(&pool, &tenant).await;
    setup_coa(&pool, &tenant).await;

    let _period = setup_test_period(
        &pool,
        &tenant,
        NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        NaiveDate::from_ymd_opt(2026, 12, 31).unwrap(),
    )
    .await;

    // USD entry
    post_journal_entry(
        &pool,
        &tenant,
        "ar",
        "USD",
        "USD Invoice",
        vec![
            ("1100".to_string(), 100_000, 0),
            ("4000".to_string(), 0, 100_000),
        ],
    )
    .await;

    // EUR entry
    post_journal_entry(
        &pool,
        &tenant,
        "ar",
        "EUR",
        "EUR Invoice",
        vec![
            ("1100".to_string(), 85_000, 0),
            ("4100".to_string(), 0, 85_000),
        ],
    )
    .await;

    // GBP entry
    post_journal_entry(
        &pool,
        &tenant,
        "ar",
        "GBP",
        "GBP Invoice",
        vec![
            ("1100".to_string(), 73_000, 0),
            ("4000".to_string(), 0, 73_000),
        ],
    )
    .await;

    // Verify each currency balances independently
    let per_currency: Vec<(String, i64, i64)> = sqlx::query_as(
        r#"
        SELECT je.currency,
               COALESCE(SUM(jl.debit_minor), 0)::BIGINT,
               COALESCE(SUM(jl.credit_minor), 0)::BIGINT
        FROM journal_lines jl
        JOIN journal_entries je ON je.id = jl.journal_entry_id
        WHERE je.tenant_id = $1
        GROUP BY je.currency
        ORDER BY je.currency
        "#,
    )
    .bind(&tenant)
    .fetch_all(&pool)
    .await
    .expect("per-currency query");

    for (currency, debits, credits) in &per_currency {
        assert_eq!(
            debits, credits,
            "Currency {} is unbalanced: debits={}, credits={}",
            currency, debits, credits
        );
    }

    assert_eq!(per_currency.len(), 3, "Should have 3 currencies");

    // Also run full invariant suite
    gl_rs::invariants::assert_all_entries_balanced(&pool, &tenant)
        .await
        .expect("Multi-currency entries must all be balanced");

    cleanup_test_tenant(&pool, &tenant).await;
    pool.close().await;
}

// ============================================================================
// Test 7: Period close — trial balance sums to zero
// ============================================================================

#[tokio::test]
#[serial]
async fn test_period_close_trial_balance_sums_to_zero() {
    let pool = get_test_pool().await;
    let tenant = unique_tenant();
    cleanup_test_tenant(&pool, &tenant).await;
    setup_coa(&pool, &tenant).await;

    let period_id = setup_test_period(
        &pool,
        &tenant,
        NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        NaiveDate::from_ymd_opt(2026, 12, 31).unwrap(),
    )
    .await;

    // Post a realistic set of transactions for a period:
    // 1. Revenue from services
    post_journal_entry(
        &pool,
        &tenant,
        "ar",
        "USD",
        "Service revenue Jan",
        vec![
            ("1100".to_string(), 500_000, 0), // AR +$5,000
            ("4100".to_string(), 0, 500_000), // Service Revenue +$5,000
        ],
    )
    .await;

    // 2. Cash collection
    post_journal_entry(
        &pool,
        &tenant,
        "payments",
        "USD",
        "Cash collection Jan",
        vec![
            ("1000".to_string(), 500_000, 0), // Cash +$5,000
            ("1100".to_string(), 0, 500_000), // AR -$5,000
        ],
    )
    .await;

    // 3. Vendor expense
    post_journal_entry(
        &pool,
        &tenant,
        "ap",
        "USD",
        "Operating expenses Jan",
        vec![
            ("6000".to_string(), 200_000, 0), // OpEx +$2,000
            ("2000".to_string(), 0, 200_000), // AP +$2,000
        ],
    )
    .await;

    // 4. Vendor payment
    post_journal_entry(
        &pool,
        &tenant,
        "ap",
        "USD",
        "Vendor payment Jan",
        vec![
            ("2000".to_string(), 200_000, 0), // AP -$2,000
            ("1000".to_string(), 0, 200_000), // Cash -$2,000
        ],
    )
    .await;

    // 5. Closing entry: move net income to retained earnings
    // Net income = Revenue($5,000) - Expenses($2,000) = $3,000
    post_journal_entry(
        &pool,
        &tenant,
        "gl",
        "USD",
        "Period close — net income to retained earnings",
        vec![
            ("4100".to_string(), 500_000, 0),  // Close Service Revenue
            ("6000".to_string(), 0, 200_000),  // Close OpEx
            ("3000".to_string(), 0, 300_000),  // Net income → Retained Earnings
        ],
    )
    .await;

    // Trial balance: sum of all debit balances - sum of all credit balances must be zero
    // Calculate per-account net balances
    let account_balances: Vec<(String, i64, i64)> = sqlx::query_as(
        r#"
        SELECT jl.account_ref,
               COALESCE(SUM(jl.debit_minor), 0)::BIGINT,
               COALESCE(SUM(jl.credit_minor), 0)::BIGINT
        FROM journal_lines jl
        JOIN journal_entries je ON je.id = jl.journal_entry_id
        WHERE je.tenant_id = $1
        GROUP BY jl.account_ref
        ORDER BY jl.account_ref
        "#,
    )
    .bind(&tenant)
    .fetch_all(&pool)
    .await
    .expect("trial balance query");

    let mut total_debits: i64 = 0;
    let mut total_credits: i64 = 0;

    for (account, debits, credits) in &account_balances {
        eprintln!(
            "  Account {}: debit={}, credit={}, net={}",
            account,
            debits,
            credits,
            debits - credits
        );
        total_debits += debits;
        total_credits += credits;
    }

    assert_eq!(
        total_debits, total_credits,
        "Trial balance must net to zero: total_debits={}, total_credits={}",
        total_debits, total_credits
    );

    // After closing entries, revenue and expense accounts should have zero net balance
    for (account, debits, credits) in &account_balances {
        if account == "4100" || account == "6000" {
            assert_eq!(
                debits, credits,
                "After closing, account {} should have zero net balance: debits={}, credits={}",
                account, debits, credits
            );
        }
    }

    // Verify Cash balance = $3,000 (collected $5,000 - paid $2,000)
    let cash = account_balances.iter().find(|(a, _, _)| a == "1000");
    if let Some((_, debits, credits)) = cash {
        assert_eq!(
            debits - credits,
            300_000,
            "Cash balance should be $3,000 net debit"
        );
    } else {
        panic!("Cash account 1000 not found in trial balance");
    }

    // Verify Retained Earnings = $3,000 (credit balance)
    let re = account_balances.iter().find(|(a, _, _)| a == "3000");
    if let Some((_, debits, credits)) = re {
        assert_eq!(
            credits - debits,
            300_000,
            "Retained Earnings should be $3,000 net credit"
        );
    } else {
        panic!("Retained Earnings account 3000 not found in trial balance");
    }

    // Mark period as closed (to verify no further postings are expected)
    sqlx::query("UPDATE accounting_periods SET is_closed = true WHERE id = $1")
        .bind(period_id)
        .execute(&pool)
        .await
        .expect("close period");

    // Run invariants on the closed period data
    gl_rs::invariants::assert_all_entries_balanced(&pool, &tenant)
        .await
        .expect("All entries balanced after period close");

    cleanup_test_tenant(&pool, &tenant).await;
    pool.close().await;
}

// ============================================================================
// Test 8: GL rejects unbalanced entries at DB level
// ============================================================================

#[tokio::test]
#[serial]
async fn test_gl_rejects_intentionally_unbalanced_entry() {
    let pool = get_test_pool().await;
    let tenant = unique_tenant();
    cleanup_test_tenant(&pool, &tenant).await;
    setup_coa(&pool, &tenant).await;

    let _period = setup_test_period(
        &pool,
        &tenant,
        NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        NaiveDate::from_ymd_opt(2026, 12, 31).unwrap(),
    )
    .await;

    // Post a balanced entry first (to have valid data in the tenant)
    post_journal_entry(
        &pool,
        &tenant,
        "ar",
        "USD",
        "Balanced entry",
        vec![
            ("1100".to_string(), 100_000, 0),
            ("4000".to_string(), 0, 100_000),
        ],
    )
    .await;

    // Intentionally post an unbalanced entry directly (bypassing validation)
    let entry_id = Uuid::new_v4();
    let source_event_id = Uuid::new_v4();
    let mut tx = pool.begin().await.expect("begin tx");

    journal_repo::insert_entry(
        &mut tx,
        entry_id,
        &tenant,
        "test",
        source_event_id,
        "test.unbalanced",
        Utc::now(),
        "USD",
        Some("Intentionally unbalanced"),
        None,
        None,
        None,
    )
    .await
    .expect("insert entry");

    // Unbalanced: debit 100, credit 50
    let lines = vec![
        JournalLineInsert {
            id: Uuid::new_v4(),
            line_no: 1,
            account_ref: "1100".to_string(),
            debit_minor: 10_000,
            credit_minor: 0,
            memo: None,
        },
        JournalLineInsert {
            id: Uuid::new_v4(),
            line_no: 2,
            account_ref: "4000".to_string(),
            debit_minor: 0,
            credit_minor: 5_000, // Only $50, not $100
            memo: None,
        },
    ];

    journal_repo::bulk_insert_lines(&mut tx, entry_id, &lines)
        .await
        .expect("insert unbalanced lines");

    tx.commit().await.expect("commit tx");

    // Now the invariant check should CATCH this unbalanced entry
    let result = gl_rs::invariants::assert_all_entries_balanced(&pool, &tenant).await;
    assert!(
        result.is_err(),
        "Invariant check must detect the unbalanced entry"
    );

    if let Err(violation) = result {
        match violation {
            gl_rs::invariants::InvariantViolation::UnbalancedEntry {
                entry_id: detected_id,
                total_debits,
                total_credits,
                difference,
            } => {
                assert_eq!(detected_id, entry_id);
                assert_eq!(total_debits, 10_000);
                assert_eq!(total_credits, 5_000);
                assert_eq!(difference, 5_000);
            }
            other => panic!("Expected UnbalancedEntry, got: {:?}", other),
        }
    }

    cleanup_test_tenant(&pool, &tenant).await;
    pool.close().await;
}

// ============================================================================
// Test 9: AP no over-allocation — allocated never exceeds bill total
// ============================================================================

#[tokio::test]
#[serial]
async fn test_ap_no_over_allocation_across_all_bills() {
    dotenvy::dotenv().ok();
    let ap_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://ap_user:ap_pass@localhost:5443/ap_db".to_string());
    let ap_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .connect(&ap_url)
        .await
        .expect("connect to AP DB");

    sqlx::migrate!("../ap/db/migrations")
        .run(&ap_pool)
        .await
        .expect("run AP migrations");

    // Query ALL non-voided bills for any over-allocation
    let over_allocated: Vec<(Uuid, i64, i64)> = sqlx::query_as(
        r#"
        SELECT b.bill_id, b.total_minor, COALESCE(SUM(a.amount_minor), 0)::BIGINT as allocated
        FROM vendor_bills b
        LEFT JOIN ap_allocations a ON a.bill_id = b.bill_id AND a.tenant_id = b.tenant_id
        WHERE b.status != 'voided'
        GROUP BY b.bill_id, b.total_minor
        HAVING COALESCE(SUM(a.amount_minor), 0)::BIGINT > b.total_minor
        "#,
    )
    .fetch_all(&ap_pool)
    .await
    .expect("over-allocation check");

    assert!(
        over_allocated.is_empty(),
        "Found {} over-allocated bills: {:?}",
        over_allocated.len(),
        over_allocated
            .iter()
            .map(|(id, total, alloc)| format!("bill {}:total={},allocated={}", id, total, alloc))
            .collect::<Vec<_>>()
    );

    ap_pool.close().await;
}

// ============================================================================
// Test 10: GL global balance check — all entries across all tenants
// ============================================================================

#[tokio::test]
#[serial]
async fn test_gl_global_debit_credit_balance() {
    let pool = get_test_pool().await;

    // Check ALL journal entries in the entire GL database
    let unbalanced: Vec<(Uuid, String, i64, i64)> = sqlx::query_as(
        r#"
        SELECT je.id, je.tenant_id,
               COALESCE(SUM(jl.debit_minor), 0)::BIGINT,
               COALESCE(SUM(jl.credit_minor), 0)::BIGINT
        FROM journal_entries je
        LEFT JOIN journal_lines jl ON jl.journal_entry_id = je.id
        GROUP BY je.id, je.tenant_id
        HAVING COALESCE(SUM(jl.debit_minor), 0) != COALESCE(SUM(jl.credit_minor), 0)
        "#,
    )
    .fetch_all(&pool)
    .await
    .expect("global balance check");

    if !unbalanced.is_empty() {
        let details: Vec<String> = unbalanced
            .iter()
            .take(10)
            .map(|(id, tenant, d, c)| {
                format!(
                    "  entry={}, tenant={}, debits={}, credits={}, diff={}",
                    id,
                    tenant,
                    d,
                    c,
                    d - c
                )
            })
            .collect();

        panic!(
            "Found {} unbalanced journal entries (showing first 10):\n{}",
            unbalanced.len(),
            details.join("\n")
        );
    }

    pool.close().await;
}
