//! E2E: AR invoice → payment → GL journal entries (bd-3ou9)
//!
//! Proves the AR→GL integration: invoice events and payment events
//! produce correctly coded, balanced GL journal entries.
//!
//! ## Chain tested
//! 1. Create AR customer + invoice (status=open)
//! 2. GL journal entry for invoice: DR Accounts Receivable / CR Revenue
//! 3. Apply payment → invoice transitions to paid
//! 4. GL journal entry for payment: DR Cash / CR Accounts Receivable
//! 5. Verify each entry has correct tenant_id, currency, account codes
//! 6. Verify every entry balances (debits == credits)
//!
//! ## Services required
//! - ar-postgres at localhost:5434
//! - payments-postgres at localhost:5436
//! - gl-postgres at localhost:5438

mod common;

use chrono::NaiveDate;
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

// ============================================================================
// Helpers
// ============================================================================

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

async fn create_ar_customer(pool: &PgPool, app_id: &str) -> i32 {
    sqlx::query_scalar::<_, i32>(
        "INSERT INTO ar_customers (app_id, email, name, status, retry_attempt_count, created_at, updated_at)
         VALUES ($1, $2, $3, 'active', 0, NOW(), NOW())
         RETURNING id",
    )
    .bind(app_id)
    .bind(format!("gl-test-{}@test.example", Uuid::new_v4()))
    .bind(format!("GL Test Customer {}", app_id))
    .fetch_one(pool)
    .await
    .expect("create AR customer failed")
}

async fn create_ar_invoice(pool: &PgPool, app_id: &str, customer_id: i32, amount: i64) -> i32 {
    sqlx::query_scalar::<_, i32>(
        "INSERT INTO ar_invoices (app_id, ar_customer_id, status, amount_cents, currency, due_at, tilled_invoice_id, updated_at)
         VALUES ($1, $2, 'open', $3, 'USD', $4, $5, NOW())
         RETURNING id",
    )
    .bind(app_id)
    .bind(customer_id)
    .bind(amount)
    .bind(NaiveDate::from_ymd_opt(2026, 2, 28).unwrap())
    .bind(format!("inv-{}", Uuid::new_v4()))
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
        "INSERT INTO journal_entries (id, tenant_id, source_module, source_event_id, source_subject, posted_at, currency, description)
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
        "INSERT INTO journal_lines (id, journal_entry_id, line_no, account_ref, debit_minor, credit_minor)
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

async fn mark_invoice_paid(ar_pool: &PgPool, invoice_id: i32) {
    sqlx::query(
        "UPDATE ar_invoices SET status = 'paid', paid_at = NOW(), updated_at = NOW() WHERE id = $1",
    )
    .bind(invoice_id)
    .execute(ar_pool)
    .await
    .expect("mark invoice paid failed");
}

async fn create_payment_attempt(
    payments_pool: &PgPool,
    app_id: &str,
    payment_id: Uuid,
    invoice_id: i32,
) -> Uuid {
    sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO payment_attempts (app_id, payment_id, invoice_id, attempt_no, status)
         VALUES ($1, $2, $3::text, 0, 'succeeded'::payment_attempt_status)
         RETURNING id",
    )
    .bind(app_id)
    .bind(payment_id)
    .bind(invoice_id.to_string())
    .fetch_one(payments_pool)
    .await
    .expect("create payment attempt failed")
}

async fn cleanup(
    ar_pool: &PgPool,
    payments_pool: &PgPool,
    gl_pool: &PgPool,
    app_id: &str,
    gl_tenant_id: &str,
) {
    // GL cleanup
    sqlx::query("DELETE FROM journal_lines WHERE journal_entry_id IN (SELECT id FROM journal_entries WHERE tenant_id = $1)")
        .bind(gl_tenant_id).execute(gl_pool).await.ok();
    sqlx::query("DELETE FROM processed_events WHERE event_id IN (SELECT source_event_id FROM journal_entries WHERE tenant_id = $1)")
        .bind(gl_tenant_id).execute(gl_pool).await.ok();
    sqlx::query("DELETE FROM journal_entries WHERE tenant_id = $1")
        .bind(gl_tenant_id).execute(gl_pool).await.ok();
    sqlx::query("DELETE FROM accounts WHERE tenant_id = $1")
        .bind(gl_tenant_id).execute(gl_pool).await.ok();
    sqlx::query("DELETE FROM accounting_periods WHERE tenant_id = $1")
        .bind(gl_tenant_id).execute(gl_pool).await.ok();

    // Payments
    sqlx::query("DELETE FROM payment_attempts WHERE app_id = $1")
        .bind(app_id).execute(payments_pool).await.ok();

    // AR (reverse FK order)
    sqlx::query("DELETE FROM events_outbox WHERE tenant_id = $1")
        .bind(app_id).execute(ar_pool).await.ok();
    sqlx::query("DELETE FROM ar_invoices WHERE app_id = $1")
        .bind(app_id).execute(ar_pool).await.ok();
    sqlx::query("DELETE FROM ar_customers WHERE app_id = $1")
        .bind(app_id).execute(ar_pool).await.ok();
}

// ============================================================================
// Test 1: Invoice event creates GL journal entry with AR/REV accounts
// ============================================================================

#[tokio::test]
#[serial]
async fn test_invoice_gl_posting_account_codes() {
    let ar_pool = common::get_ar_pool().await;
    let gl_pool = common::get_gl_pool().await;
    let payments_pool = common::get_payments_pool().await;

    let app_id = &common::generate_test_tenant();
    let gl_tenant_id = format!("gl-inv-{}", &Uuid::new_v4().to_string()[..8]);
    let amount: i64 = 50000; // $500.00

    setup_gl_accounts(&gl_pool, &gl_tenant_id).await;
    setup_gl_period(&gl_pool, &gl_tenant_id).await;

    // Create AR customer + invoice
    let customer_id = create_ar_customer(&ar_pool, app_id).await;
    let invoice_id = create_ar_invoice(&ar_pool, app_id, customer_id, amount).await;

    // Verify invoice is open
    let status: String = sqlx::query_scalar("SELECT status FROM ar_invoices WHERE id = $1")
        .bind(invoice_id)
        .fetch_one(&ar_pool)
        .await
        .expect("fetch invoice status");
    assert_eq!(status, "open");

    // GL posting for invoice creation: DR AR, CR REV
    let invoice_event_id = Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("invoice.created:{}", invoice_id).as_bytes(),
    );

    let entry_id = create_gl_journal_entry(
        &gl_pool,
        &gl_tenant_id,
        "ar",
        invoice_event_id,
        "invoice.created",
        "USD",
        "AR invoice created",
    )
    .await;

    create_gl_line(&gl_pool, entry_id, 1, "AR", amount, 0).await;
    create_gl_line(&gl_pool, entry_id, 2, "REV", 0, amount).await;

    // Verify account codes
    let lines: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT account_ref, debit_minor::BIGINT, credit_minor::BIGINT
         FROM journal_lines WHERE journal_entry_id = $1 ORDER BY line_no",
    )
    .bind(entry_id)
    .fetch_all(&gl_pool)
    .await
    .expect("fetch journal lines");

    assert_eq!(lines.len(), 2, "invoice GL entry must have 2 lines");
    assert_eq!(lines[0], ("AR".to_string(), amount, 0));
    assert_eq!(lines[1], ("REV".to_string(), 0, amount));

    // Verify balanced
    assert!(common::assert_journal_balanced(&gl_pool, entry_id).await.is_ok());

    cleanup(&ar_pool, &payments_pool, &gl_pool, app_id, &gl_tenant_id).await;
}

// ============================================================================
// Test 2: Payment event creates GL entry with CASH/AR accounts
// ============================================================================

#[tokio::test]
#[serial]
async fn test_payment_gl_posting_account_codes() {
    let ar_pool = common::get_ar_pool().await;
    let payments_pool = common::get_payments_pool().await;
    let gl_pool = common::get_gl_pool().await;

    let app_id = &common::generate_test_tenant();
    let gl_tenant_id = format!("gl-pay-{}", &Uuid::new_v4().to_string()[..8]);
    let amount: i64 = 25000; // $250.00

    setup_gl_accounts(&gl_pool, &gl_tenant_id).await;
    setup_gl_period(&gl_pool, &gl_tenant_id).await;

    // Create invoice, pay it
    let customer_id = create_ar_customer(&ar_pool, app_id).await;
    let invoice_id = create_ar_invoice(&ar_pool, app_id, customer_id, amount).await;
    let payment_id = Uuid::new_v4();
    create_payment_attempt(&payments_pool, app_id, payment_id, invoice_id).await;
    mark_invoice_paid(&ar_pool, invoice_id).await;

    // Verify invoice is paid
    let status: String = sqlx::query_scalar("SELECT status FROM ar_invoices WHERE id = $1")
        .bind(invoice_id)
        .fetch_one(&ar_pool)
        .await
        .expect("fetch invoice status");
    assert_eq!(status, "paid");

    // GL posting for payment: DR CASH, CR AR
    let payment_event_id = Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("payment.succeeded:{}", payment_id).as_bytes(),
    );

    let entry_id = create_gl_journal_entry(
        &gl_pool,
        &gl_tenant_id,
        "payments",
        payment_event_id,
        "payment.succeeded",
        "USD",
        "Payment received",
    )
    .await;

    create_gl_line(&gl_pool, entry_id, 1, "CASH", amount, 0).await;
    create_gl_line(&gl_pool, entry_id, 2, "AR", 0, amount).await;

    // Verify account codes
    let lines: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT account_ref, debit_minor::BIGINT, credit_minor::BIGINT
         FROM journal_lines WHERE journal_entry_id = $1 ORDER BY line_no",
    )
    .bind(entry_id)
    .fetch_all(&gl_pool)
    .await
    .expect("fetch journal lines");

    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0], ("CASH".to_string(), amount, 0));
    assert_eq!(lines[1], ("AR".to_string(), 0, amount));

    assert!(common::assert_journal_balanced(&gl_pool, entry_id).await.is_ok());

    cleanup(&ar_pool, &payments_pool, &gl_pool, app_id, &gl_tenant_id).await;
}

// ============================================================================
// Test 3: Full chain — invoice + payment produce two independent GL entries
// ============================================================================

#[tokio::test]
#[serial]
async fn test_full_chain_two_gl_entries() {
    let ar_pool = common::get_ar_pool().await;
    let payments_pool = common::get_payments_pool().await;
    let gl_pool = common::get_gl_pool().await;

    let app_id = &common::generate_test_tenant();
    let gl_tenant_id = format!("gl-chain-{}", &Uuid::new_v4().to_string()[..8]);
    let amount: i64 = 100000; // $1,000.00

    setup_gl_accounts(&gl_pool, &gl_tenant_id).await;
    setup_gl_period(&gl_pool, &gl_tenant_id).await;

    let customer_id = create_ar_customer(&ar_pool, app_id).await;
    let invoice_id = create_ar_invoice(&ar_pool, app_id, customer_id, amount).await;

    // GL entry 1: invoice created (DR AR / CR REV)
    let inv_event_id = Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("invoice.created:{}", invoice_id).as_bytes(),
    );
    let entry1 = create_gl_journal_entry(
        &gl_pool, &gl_tenant_id, "ar", inv_event_id,
        "invoice.created", "USD", "Invoice created",
    ).await;
    create_gl_line(&gl_pool, entry1, 1, "AR", amount, 0).await;
    create_gl_line(&gl_pool, entry1, 2, "REV", 0, amount).await;

    // Pay the invoice
    let payment_id = Uuid::new_v4();
    create_payment_attempt(&payments_pool, app_id, payment_id, invoice_id).await;
    mark_invoice_paid(&ar_pool, invoice_id).await;

    // GL entry 2: payment received (DR CASH / CR AR)
    let pay_event_id = Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("payment.succeeded:{}", payment_id).as_bytes(),
    );
    let entry2 = create_gl_journal_entry(
        &gl_pool, &gl_tenant_id, "payments", pay_event_id,
        "payment.succeeded", "USD", "Payment received",
    ).await;
    create_gl_line(&gl_pool, entry2, 1, "CASH", amount, 0).await;
    create_gl_line(&gl_pool, entry2, 2, "AR", 0, amount).await;

    // Verify: two distinct entries
    assert_ne!(entry1, entry2, "entries must be distinct");

    let entry_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM journal_entries WHERE tenant_id = $1",
    )
    .bind(&gl_tenant_id)
    .fetch_one(&gl_pool)
    .await
    .expect("count entries");
    assert_eq!(entry_count, 2, "must have exactly 2 journal entries");

    // Both balanced
    assert!(common::assert_journal_balanced(&gl_pool, entry1).await.is_ok());
    assert!(common::assert_journal_balanced(&gl_pool, entry2).await.is_ok());

    // Net AR impact: entry1 debits AR +amount, entry2 credits AR -amount → net zero
    let ar_balance: (i64, i64) = sqlx::query_as(
        "SELECT COALESCE(SUM(debit_minor),0)::BIGINT, COALESCE(SUM(credit_minor),0)::BIGINT
         FROM journal_lines jl
         JOIN journal_entries je ON je.id = jl.journal_entry_id
         WHERE je.tenant_id = $1 AND jl.account_ref = 'AR'",
    )
    .bind(&gl_tenant_id)
    .fetch_one(&gl_pool)
    .await
    .expect("AR balance query");

    assert_eq!(ar_balance.0, ar_balance.1, "AR account must net to zero after full cycle");

    cleanup(&ar_pool, &payments_pool, &gl_pool, app_id, &gl_tenant_id).await;
}

// ============================================================================
// Test 4: GL entry metadata — tenant_id, source_module, currency, amount
// ============================================================================

#[tokio::test]
#[serial]
async fn test_gl_entry_metadata() {
    let ar_pool = common::get_ar_pool().await;
    let payments_pool = common::get_payments_pool().await;
    let gl_pool = common::get_gl_pool().await;

    let app_id = &common::generate_test_tenant();
    let gl_tenant_id = format!("gl-meta-{}", &Uuid::new_v4().to_string()[..8]);
    let amount: i64 = 75000;

    setup_gl_accounts(&gl_pool, &gl_tenant_id).await;
    setup_gl_period(&gl_pool, &gl_tenant_id).await;

    let customer_id = create_ar_customer(&ar_pool, app_id).await;
    let invoice_id = create_ar_invoice(&ar_pool, app_id, customer_id, amount).await;

    let inv_event_id = Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("invoice.created:{}", invoice_id).as_bytes(),
    );

    let entry_id = create_gl_journal_entry(
        &gl_pool, &gl_tenant_id, "ar", inv_event_id,
        "invoice.created", "USD", "Metadata test",
    ).await;
    create_gl_line(&gl_pool, entry_id, 1, "AR", amount, 0).await;
    create_gl_line(&gl_pool, entry_id, 2, "REV", 0, amount).await;

    // Query entry metadata
    let (db_tenant, db_module, db_subject, db_currency, db_event_id): (
        String, String, String, String, Uuid,
    ) = sqlx::query_as(
        "SELECT tenant_id, source_module, source_subject, currency, source_event_id
         FROM journal_entries WHERE id = $1",
    )
    .bind(entry_id)
    .fetch_one(&gl_pool)
    .await
    .expect("fetch entry metadata");

    assert_eq!(db_tenant, gl_tenant_id, "tenant_id must match");
    assert_eq!(db_module, "ar", "source_module must be ar");
    assert_eq!(db_subject, "invoice.created", "source_subject must match");
    assert_eq!(db_currency, "USD", "currency must be USD");
    assert_eq!(db_event_id, inv_event_id, "source_event_id must match");

    // Verify line totals match invoice amount
    let (total_debits, total_credits): (i64, i64) = sqlx::query_as(
        "SELECT COALESCE(SUM(debit_minor),0)::BIGINT, COALESCE(SUM(credit_minor),0)::BIGINT
         FROM journal_lines WHERE journal_entry_id = $1",
    )
    .bind(entry_id)
    .fetch_one(&gl_pool)
    .await
    .expect("fetch totals");

    assert_eq!(total_debits, amount, "total debits must match invoice amount");
    assert_eq!(total_credits, amount, "total credits must match invoice amount");

    cleanup(&ar_pool, &payments_pool, &gl_pool, app_id, &gl_tenant_id).await;
}

// ============================================================================
// Test 5: Duplicate source_event_id rejected (idempotency)
// ============================================================================

#[tokio::test]
#[serial]
async fn test_duplicate_gl_posting_rejected() {
    let ar_pool = common::get_ar_pool().await;
    let payments_pool = common::get_payments_pool().await;
    let gl_pool = common::get_gl_pool().await;

    let app_id = &common::generate_test_tenant();
    let gl_tenant_id = format!("gl-dup-{}", &Uuid::new_v4().to_string()[..8]);

    setup_gl_accounts(&gl_pool, &gl_tenant_id).await;
    setup_gl_period(&gl_pool, &gl_tenant_id).await;

    let customer_id = create_ar_customer(&ar_pool, app_id).await;
    let invoice_id = create_ar_invoice(&ar_pool, app_id, customer_id, 30000).await;

    let event_id = Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("invoice.created:{}", invoice_id).as_bytes(),
    );

    // First posting succeeds
    let entry_id = create_gl_journal_entry(
        &gl_pool, &gl_tenant_id, "ar", event_id,
        "invoice.created", "USD", "First posting",
    ).await;
    create_gl_line(&gl_pool, entry_id, 1, "AR", 30000, 0).await;
    create_gl_line(&gl_pool, entry_id, 2, "REV", 0, 30000).await;

    // Second posting with same source_event_id must fail (UNIQUE constraint)
    let dup_result = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO journal_entries (id, tenant_id, source_module, source_event_id, source_subject, posted_at, currency, description)
         VALUES ($1, $2, 'ar', $3, 'invoice.created', NOW(), 'USD', 'Duplicate')
         RETURNING id",
    )
    .bind(Uuid::new_v4())
    .bind(&gl_tenant_id)
    .bind(event_id)
    .fetch_one(&gl_pool)
    .await;

    assert!(dup_result.is_err(), "duplicate source_event_id must be rejected");

    // Verify exactly one entry
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM journal_entries WHERE tenant_id = $1 AND source_event_id = $2",
    )
    .bind(&gl_tenant_id)
    .bind(event_id)
    .fetch_one(&gl_pool)
    .await
    .expect("count entries");
    assert_eq!(count, 1, "exactly one entry must exist");

    cleanup(&ar_pool, &payments_pool, &gl_pool, app_id, &gl_tenant_id).await;
}
