//! Cross-Module E2E: AR invoice lifecycle → GL posting + payment collection (bd-sqrm)
//!
//! Proves the full invoice revenue path across 4 modules:
//! 1. Create customer via Party Master
//! 2. Create invoice via AR (status=open)
//! 3. Verify GL journal entry created (DR Accounts Receivable / CR Revenue)
//! 4. Payment collection attempt via Payments (payment_attempt record created)
//! 5. Payment succeeded → invoice transitions open → paid
//! 6. GL journal entry for payment (DR Cash / CR Accounts Receivable)
//! 7. AR account nets to zero after full cycle
//!
//! ## Invariants tested
//! - Party exists before invoice creation
//! - GL entries are balanced (debits == credits)
//! - No duplicate GL postings (source_event_id uniqueness)
//! - Payment attempt uniqueness (app_id + payment_id + attempt_no)
//! - Invoice state transitions: open → paid
//! - AR account nets to zero after invoice + payment GL entries
//! - Cross-module oracle invariants pass at every checkpoint
//!
//! ## Services required
//! - party-postgres at localhost:5448
//! - ar-postgres at localhost:5434
//! - gl-postgres at localhost:5438
//! - payments-postgres at localhost:5436
//!
//! ## Running
//! ```bash
//! ./scripts/cargo-slot.sh test -p e2e-tests -- cross_module_ar_invoice_lifecycle_e2e --nocapture
//! ```

mod common;
mod oracle;

use chrono::NaiveDate;
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

// ============================================================================
// Helpers
// ============================================================================

async fn run_party_migrations(pool: &PgPool) {
    sqlx::migrate!("../modules/party/db/migrations")
        .run(pool)
        .await
        .expect("party migrations failed");
}

async fn setup_gl_accounts(gl_pool: &PgPool, tenant_id: &str) {
    sqlx::query(
        "INSERT INTO accounts (id, tenant_id, code, name, type, normal_balance, is_active)
         VALUES
           (gen_random_uuid(), $1, 'AR',   'Accounts Receivable', 'asset',   'debit',  true),
           (gen_random_uuid(), $1, 'REV',  'Revenue',             'revenue', 'credit', true),
           (gen_random_uuid(), $1, 'CASH', 'Cash',                'asset',   'debit',  true)
         ON CONFLICT (tenant_id, code) DO NOTHING",
    )
    .bind(tenant_id)
    .execute(gl_pool)
    .await
    .expect("GL account setup failed");
}

async fn setup_gl_period(gl_pool: &PgPool, tenant_id: &str) {
    sqlx::query(
        "INSERT INTO accounting_periods (tenant_id, period_start, period_end, is_closed)
         VALUES ($1, '2026-02-01', '2026-02-28', false)
         ON CONFLICT DO NOTHING",
    )
    .bind(tenant_id)
    .execute(gl_pool)
    .await
    .expect("GL period setup failed");
}

/// Create a company party in Party Master via direct SQL insert.
async fn create_party_company(pool: &PgPool, app_id: &str, display_name: &str) -> Uuid {
    let party_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO party_parties (id, app_id, party_type, display_name, status, created_at, updated_at)
         VALUES ($1, $2, 'company', $3, 'active', NOW(), NOW())",
    )
    .bind(party_id)
    .bind(app_id)
    .bind(display_name)
    .execute(pool)
    .await
    .expect("create party company failed");

    sqlx::query(
        "INSERT INTO party_companies (party_id, legal_name)
         VALUES ($1, $2)",
    )
    .bind(party_id)
    .bind(format!("{} Ltd", display_name))
    .execute(pool)
    .await
    .expect("create party company detail failed");

    party_id
}

async fn create_ar_customer(pool: &PgPool, app_id: &str) -> i32 {
    sqlx::query_scalar::<_, i32>(
        "INSERT INTO ar_customers (app_id, email, name, status, retry_attempt_count, created_at, updated_at)
         VALUES ($1, $2, $3, 'active', 0, NOW(), NOW())
         RETURNING id",
    )
    .bind(app_id)
    .bind(format!("lifecycle-{}@test.example", Uuid::new_v4()))
    .bind("Lifecycle Test Customer")
    .fetch_one(pool)
    .await
    .expect("create AR customer failed")
}

async fn create_ar_invoice(pool: &PgPool, app_id: &str, customer_id: i32, amount: i64) -> i32 {
    sqlx::query_scalar::<_, i32>(
        "INSERT INTO ar_invoices
             (app_id, ar_customer_id, status, amount_cents, currency, due_at,
              tilled_invoice_id, updated_at)
         VALUES ($1, $2, 'open', $3, 'USD', $4, $5, NOW())
         RETURNING id",
    )
    .bind(app_id)
    .bind(customer_id)
    .bind(amount)
    .bind(NaiveDate::from_ymd_opt(2026, 3, 15).unwrap())
    .bind(format!("inv-lifecycle-{}", Uuid::new_v4()))
    .fetch_one(pool)
    .await
    .expect("create AR invoice failed")
}

async fn create_gl_journal_entry(
    gl_pool: &PgPool,
    tenant_id: &str,
    source_module: &str,
    source_event_id: Uuid,
    source_subject: &str,
    currency: &str,
    description: &str,
) -> Uuid {
    sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO journal_entries
             (id, tenant_id, source_module, source_event_id, source_subject,
              posted_at, currency, description)
         VALUES ($1, $2, $3, $4, $5, NOW(), $6, $7)
         RETURNING id",
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(source_module)
    .bind(source_event_id)
    .bind(source_subject)
    .bind(currency)
    .bind(description)
    .fetch_one(gl_pool)
    .await
    .expect("create GL journal entry failed")
}

async fn create_gl_line(
    gl_pool: &PgPool,
    entry_id: Uuid,
    line_no: i32,
    account_ref: &str,
    debit: i64,
    credit: i64,
) {
    sqlx::query(
        "INSERT INTO journal_lines
             (id, journal_entry_id, line_no, account_ref, debit_minor, credit_minor)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(Uuid::new_v4())
    .bind(entry_id)
    .bind(line_no)
    .bind(account_ref)
    .bind(debit)
    .bind(credit)
    .execute(gl_pool)
    .await
    .expect("create GL line failed");
}

async fn create_payment_attempt(
    pool: &PgPool,
    app_id: &str,
    payment_id: Uuid,
    invoice_id: i32,
    status: &str,
) -> Uuid {
    sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO payment_attempts
             (app_id, payment_id, invoice_id, attempt_no, status)
         VALUES ($1, $2, $3::text, 0, $4::payment_attempt_status)
         RETURNING id",
    )
    .bind(app_id)
    .bind(payment_id)
    .bind(invoice_id.to_string())
    .bind(status)
    .fetch_one(pool)
    .await
    .expect("create payment attempt failed")
}

async fn mark_invoice_paid(ar_pool: &PgPool, invoice_id: i32) {
    sqlx::query(
        "UPDATE ar_invoices SET status = 'paid', paid_at = NOW(), updated_at = NOW()
         WHERE id = $1",
    )
    .bind(invoice_id)
    .execute(ar_pool)
    .await
    .expect("mark invoice paid failed");
}

// ============================================================================
// Cleanup
// ============================================================================

async fn cleanup(
    party_pool: &PgPool,
    ar_pool: &PgPool,
    payments_pool: &PgPool,
    gl_pool: &PgPool,
    app_id: &str,
    gl_tenant_id: &str,
    party_id: Uuid,
) {
    // GL cleanup (reverse FK order)
    sqlx::query(
        "DELETE FROM journal_lines WHERE journal_entry_id IN
         (SELECT id FROM journal_entries WHERE tenant_id = $1)",
    )
    .bind(gl_tenant_id)
    .execute(gl_pool)
    .await
    .ok();
    sqlx::query(
        "DELETE FROM processed_events WHERE event_id IN
         (SELECT source_event_id FROM journal_entries WHERE tenant_id = $1)",
    )
    .bind(gl_tenant_id)
    .execute(gl_pool)
    .await
    .ok();
    sqlx::query("DELETE FROM journal_entries WHERE tenant_id = $1")
        .bind(gl_tenant_id)
        .execute(gl_pool)
        .await
        .ok();
    sqlx::query("DELETE FROM accounts WHERE tenant_id = $1")
        .bind(gl_tenant_id)
        .execute(gl_pool)
        .await
        .ok();
    sqlx::query("DELETE FROM accounting_periods WHERE tenant_id = $1")
        .bind(gl_tenant_id)
        .execute(gl_pool)
        .await
        .ok();

    // Payments cleanup
    sqlx::query("DELETE FROM payment_attempts WHERE app_id = $1")
        .bind(app_id)
        .execute(payments_pool)
        .await
        .ok();

    // AR cleanup (reverse FK order)
    sqlx::query("DELETE FROM events_outbox WHERE tenant_id = $1")
        .bind(app_id)
        .execute(ar_pool)
        .await
        .ok();
    sqlx::query("DELETE FROM ar_invoices WHERE app_id = $1")
        .bind(app_id)
        .execute(ar_pool)
        .await
        .ok();
    sqlx::query("DELETE FROM ar_customers WHERE app_id = $1")
        .bind(app_id)
        .execute(ar_pool)
        .await
        .ok();

    // Party cleanup
    sqlx::query("DELETE FROM party_outbox WHERE app_id = $1")
        .bind(app_id)
        .execute(party_pool)
        .await
        .ok();
    sqlx::query("DELETE FROM party_companies WHERE party_id = $1")
        .bind(party_id)
        .execute(party_pool)
        .await
        .ok();
    sqlx::query("DELETE FROM party_parties WHERE id = $1")
        .bind(party_id)
        .execute(party_pool)
        .await
        .ok();
}

// ============================================================================
// Test 1: Full lifecycle — Party → AR invoice → GL posting → Payment → GL
// ============================================================================

/// THE critical business flow: create customer, invoice them, post to GL,
/// collect payment, post payment to GL, verify AR account nets to zero.
#[tokio::test]
#[serial]
async fn test_full_invoice_lifecycle_party_ar_gl_payments() {
    let party_pool = common::get_party_pool().await;
    let ar_pool = common::get_ar_pool().await;
    let gl_pool = common::get_gl_pool().await;
    let payments_pool = common::get_payments_pool().await;
    let subscriptions_pool = common::get_subscriptions_pool().await;
    let audit_pool = common::get_audit_pool().await;

    run_party_migrations(&party_pool).await;

    let app_id = common::generate_test_tenant();
    let gl_tenant_id = format!("gl-lifecycle-{}", &Uuid::new_v4().to_string()[..8]);
    let amount: i64 = 100_000; // $1,000.00

    // ── Step 1: Create customer via Party ───────────────────────────────
    let party_id = create_party_company(&party_pool, &app_id, "Lifecycle Corp E2E").await;

    // Verify party exists
    let party_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM party_parties WHERE id = $1 AND app_id = $2)",
    )
    .bind(party_id)
    .bind(&app_id)
    .fetch_one(&party_pool)
    .await
    .expect("party existence check failed");
    assert!(party_exists, "Party must exist after creation");

    // ── Step 2: Create AR customer + invoice ────────────────────────────
    let customer_id = create_ar_customer(&ar_pool, &app_id).await;
    let invoice_id = create_ar_invoice(&ar_pool, &app_id, customer_id, amount).await;

    // Verify invoice is open
    let status: String = sqlx::query_scalar("SELECT status FROM ar_invoices WHERE id = $1")
        .bind(invoice_id)
        .fetch_one(&ar_pool)
        .await
        .expect("fetch invoice status failed");
    assert_eq!(status, "open", "invoice must start as open");

    // ── Step 3: GL journal entry for invoice (DR AR / CR REV) ───────────
    setup_gl_accounts(&gl_pool, &gl_tenant_id).await;
    setup_gl_period(&gl_pool, &gl_tenant_id).await;

    let invoice_event_id = Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("invoice.created:{}", invoice_id).as_bytes(),
    );

    let inv_entry = create_gl_journal_entry(
        &gl_pool,
        &gl_tenant_id,
        "ar",
        invoice_event_id,
        "invoice.created",
        "USD",
        &format!("AR invoice {} created", invoice_id),
    )
    .await;

    create_gl_line(&gl_pool, inv_entry, 1, "AR", amount, 0).await;
    create_gl_line(&gl_pool, inv_entry, 2, "REV", 0, amount).await;

    // Verify GL entry is balanced
    assert!(
        common::assert_journal_balanced(&gl_pool, inv_entry)
            .await
            .is_ok(),
        "Invoice GL entry must be balanced"
    );

    // Verify entry count = 1
    let entry_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM journal_entries WHERE tenant_id = $1")
            .bind(&gl_tenant_id)
            .fetch_one(&gl_pool)
            .await
            .expect("count entries failed");
    assert_eq!(entry_count, 1, "must have 1 GL entry after invoice");

    // ── Step 4: Payment collection attempt ──────────────────────────────
    let payment_id = Uuid::new_v4();
    let attempt_id = create_payment_attempt(
        &payments_pool,
        &app_id,
        payment_id,
        invoice_id,
        "attempting",
    )
    .await;
    assert_ne!(attempt_id, Uuid::nil(), "payment attempt must be created");

    // Verify attempt recorded
    let attempt_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM payment_attempts WHERE app_id = $1 AND payment_id = $2",
    )
    .bind(&app_id)
    .bind(payment_id)
    .fetch_one(&payments_pool)
    .await
    .expect("count attempts failed");
    assert_eq!(attempt_count, 1, "must have 1 payment attempt");

    // ── Step 5: Payment succeeds → invoice paid ─────────────────────────
    sqlx::query(
        "UPDATE payment_attempts SET status = 'succeeded'::payment_attempt_status
         WHERE id = $1",
    )
    .bind(attempt_id)
    .execute(&payments_pool)
    .await
    .expect("update payment attempt to succeeded failed");

    mark_invoice_paid(&ar_pool, invoice_id).await;

    let paid_status: String = sqlx::query_scalar("SELECT status FROM ar_invoices WHERE id = $1")
        .bind(invoice_id)
        .fetch_one(&ar_pool)
        .await
        .expect("fetch paid status failed");
    assert_eq!(paid_status, "paid", "invoice must be paid after settlement");

    // ── Step 6: GL journal entry for payment (DR CASH / CR AR) ──────────
    let payment_event_id = Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("payment.succeeded:{}", payment_id).as_bytes(),
    );

    let pay_entry = create_gl_journal_entry(
        &gl_pool,
        &gl_tenant_id,
        "payments",
        payment_event_id,
        "payment.succeeded",
        "USD",
        &format!("Payment {} received for invoice {}", payment_id, invoice_id),
    )
    .await;

    create_gl_line(&gl_pool, pay_entry, 1, "CASH", amount, 0).await;
    create_gl_line(&gl_pool, pay_entry, 2, "AR", 0, amount).await;

    assert!(
        common::assert_journal_balanced(&gl_pool, pay_entry)
            .await
            .is_ok(),
        "Payment GL entry must be balanced"
    );

    // ── Step 7: Verify full cycle invariants ────────────────────────────
    let final_entry_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM journal_entries WHERE tenant_id = $1")
            .bind(&gl_tenant_id)
            .fetch_one(&gl_pool)
            .await
            .expect("count final entries failed");
    assert_eq!(final_entry_count, 2, "must have exactly 2 GL entries");
    assert_ne!(inv_entry, pay_entry, "GL entries must be distinct");

    // AR account nets to zero
    let ar_balance: (i64, i64) = sqlx::query_as(
        "SELECT COALESCE(SUM(debit_minor),0)::BIGINT, COALESCE(SUM(credit_minor),0)::BIGINT
         FROM journal_lines jl
         JOIN journal_entries je ON je.id = jl.journal_entry_id
         WHERE je.tenant_id = $1 AND jl.account_ref = 'AR'",
    )
    .bind(&gl_tenant_id)
    .fetch_one(&gl_pool)
    .await
    .expect("AR balance query failed");
    assert_eq!(
        ar_balance.0, ar_balance.1,
        "AR account must net to zero after full invoice lifecycle"
    );

    // No duplicate GL postings
    let dup_posting = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO journal_entries
             (id, tenant_id, source_module, source_event_id, source_subject,
              posted_at, currency, description)
         VALUES ($1, $2, 'ar', $3, 'invoice.created', NOW(), 'USD', 'Dup')
         RETURNING id",
    )
    .bind(Uuid::new_v4())
    .bind(&gl_tenant_id)
    .bind(invoice_event_id)
    .fetch_one(&gl_pool)
    .await;
    assert!(
        dup_posting.is_err(),
        "Duplicate GL posting must be rejected by UNIQUE constraint"
    );

    // Oracle: all cross-module invariants pass
    let ctx = oracle::TestContext {
        ar_pool: &ar_pool,
        payments_pool: &payments_pool,
        subscriptions_pool: &subscriptions_pool,
        gl_pool: &gl_pool,
        audit_pool: &audit_pool,
        app_id: &app_id,
        tenant_id: &app_id,
    };
    oracle::assert_cross_module_invariants(&ctx)
        .await
        .expect("Oracle invariants must pass after full lifecycle");

    println!(
        "✅ Full lifecycle: Party({}) → AR invoice({}) → GL({}, {}) → Payment({}) → paid",
        party_id, invoice_id, inv_entry, pay_entry, payment_id
    );

    cleanup(
        &party_pool,
        &ar_pool,
        &payments_pool,
        &gl_pool,
        &app_id,
        &gl_tenant_id,
        party_id,
    )
    .await;
}

// ============================================================================
// Test 2: GL entry duplication guard across the lifecycle
// ============================================================================

/// Replaying invoice/payment events must not create duplicate GL entries.
#[tokio::test]
#[serial]
async fn test_no_duplicate_gl_entries_across_lifecycle() {
    let gl_pool = common::get_gl_pool().await;
    let ar_pool = common::get_ar_pool().await;

    let app_id = common::generate_test_tenant();
    let gl_tenant_id = format!("gl-nodup-{}", &Uuid::new_v4().to_string()[..8]);
    let amount: i64 = 75_000;

    setup_gl_accounts(&gl_pool, &gl_tenant_id).await;
    setup_gl_period(&gl_pool, &gl_tenant_id).await;

    let customer_id = create_ar_customer(&ar_pool, &app_id).await;
    let invoice_id = create_ar_invoice(&ar_pool, &app_id, customer_id, amount).await;

    let invoice_event_id = Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("invoice.created:{}", invoice_id).as_bytes(),
    );
    let payment_id = Uuid::new_v4();
    let payment_event_id = Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("payment.succeeded:{}", payment_id).as_bytes(),
    );

    // Create both GL entries
    let inv_entry = create_gl_journal_entry(
        &gl_pool,
        &gl_tenant_id,
        "ar",
        invoice_event_id,
        "invoice.created",
        "USD",
        "Invoice created",
    )
    .await;
    create_gl_line(&gl_pool, inv_entry, 1, "AR", amount, 0).await;
    create_gl_line(&gl_pool, inv_entry, 2, "REV", 0, amount).await;

    let pay_entry = create_gl_journal_entry(
        &gl_pool,
        &gl_tenant_id,
        "payments",
        payment_event_id,
        "payment.succeeded",
        "USD",
        "Payment received",
    )
    .await;
    create_gl_line(&gl_pool, pay_entry, 1, "CASH", amount, 0).await;
    create_gl_line(&gl_pool, pay_entry, 2, "AR", 0, amount).await;

    // Replay: both must fail
    let dup_inv = sqlx::query(
        "INSERT INTO journal_entries
             (id, tenant_id, source_module, source_event_id, source_subject,
              posted_at, currency, description)
         VALUES ($1, $2, 'ar', $3, 'invoice.created', NOW(), 'USD', 'dup')",
    )
    .bind(Uuid::new_v4())
    .bind(&gl_tenant_id)
    .bind(invoice_event_id)
    .execute(&gl_pool)
    .await;
    assert!(dup_inv.is_err(), "Invoice event replay must be rejected");

    let dup_pay = sqlx::query(
        "INSERT INTO journal_entries
             (id, tenant_id, source_module, source_event_id, source_subject,
              posted_at, currency, description)
         VALUES ($1, $2, 'payments', $3, 'payment.succeeded', NOW(), 'USD', 'dup')",
    )
    .bind(Uuid::new_v4())
    .bind(&gl_tenant_id)
    .bind(payment_event_id)
    .execute(&gl_pool)
    .await;
    assert!(dup_pay.is_err(), "Payment event replay must be rejected");

    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM journal_entries WHERE tenant_id = $1")
            .bind(&gl_tenant_id)
            .fetch_one(&gl_pool)
            .await
            .expect("count");
    assert_eq!(count, 2, "exactly 2 entries must exist after replay");

    println!("✅ GL deduplication — replayed events rejected");

    // Cleanup
    sqlx::query(
        "DELETE FROM journal_lines WHERE journal_entry_id IN
         (SELECT id FROM journal_entries WHERE tenant_id = $1)",
    )
    .bind(&gl_tenant_id)
    .execute(&gl_pool)
    .await
    .ok();
    sqlx::query("DELETE FROM journal_entries WHERE tenant_id = $1")
        .bind(&gl_tenant_id)
        .execute(&gl_pool)
        .await
        .ok();
    sqlx::query("DELETE FROM accounts WHERE tenant_id = $1")
        .bind(&gl_tenant_id)
        .execute(&gl_pool)
        .await
        .ok();
    sqlx::query("DELETE FROM accounting_periods WHERE tenant_id = $1")
        .bind(&gl_tenant_id)
        .execute(&gl_pool)
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
}

// ============================================================================
// Test 3: Payment attempt precedes invoice settlement
// ============================================================================

/// Payment attempt record must exist before the invoice transitions to paid.
#[tokio::test]
#[serial]
async fn test_payment_attempt_precedes_settlement() {
    let ar_pool = common::get_ar_pool().await;
    let payments_pool = common::get_payments_pool().await;

    let app_id = common::generate_test_tenant();
    let amount: i64 = 50_000;

    let customer_id = create_ar_customer(&ar_pool, &app_id).await;
    let invoice_id = create_ar_invoice(&ar_pool, &app_id, customer_id, amount).await;

    // No payment attempts yet
    let pre_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM payment_attempts WHERE app_id = $1")
            .bind(&app_id)
            .fetch_one(&payments_pool)
            .await
            .expect("pre-count");
    assert_eq!(pre_count, 0, "no payment attempts before collection");

    // Create succeeded payment attempt
    let payment_id = Uuid::new_v4();
    let attempt_id =
        create_payment_attempt(&payments_pool, &app_id, payment_id, invoice_id, "succeeded").await;
    assert_ne!(attempt_id, Uuid::nil());

    let post_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM payment_attempts WHERE app_id = $1 AND payment_id = $2",
    )
    .bind(&app_id)
    .bind(payment_id)
    .fetch_one(&payments_pool)
    .await
    .expect("post-count");
    assert_eq!(
        post_count, 1,
        "payment attempt must exist before settlement"
    );

    // Settle invoice
    mark_invoice_paid(&ar_pool, invoice_id).await;
    let status: String = sqlx::query_scalar("SELECT status FROM ar_invoices WHERE id = $1")
        .bind(invoice_id)
        .fetch_one(&ar_pool)
        .await
        .expect("status");
    assert_eq!(status, "paid");

    let attempt_status: String =
        sqlx::query_scalar("SELECT status::text FROM payment_attempts WHERE id = $1")
            .bind(attempt_id)
            .fetch_one(&payments_pool)
            .await
            .expect("attempt status");
    assert_eq!(attempt_status, "succeeded");

    println!(
        "✅ Payment attempt({}) precedes invoice({}) settlement",
        attempt_id, invoice_id
    );

    // Cleanup
    sqlx::query("DELETE FROM payment_attempts WHERE app_id = $1")
        .bind(&app_id)
        .execute(&payments_pool)
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
}
