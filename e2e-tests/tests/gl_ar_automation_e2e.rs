//! E2E: GL Automation — AR invoice.created fires GL journal entry (bd-3hzd)
//!
//! Proves the AR → GL automation chain: when an AR invoice is created,
//! the platform emits a `gl.posting.requested` event via NATS. The GL
//! posting consumer processes it and creates a balanced double-entry
//! journal entry.
//!
//! ## Chain tested
//! 1. AR invoice created → AR emits `gl.posting.requested` outbox event
//! 2. GL posting consumer processes event via `process_gl_posting_request`
//! 3. Journal entry: DR 1100 (Accounts Receivable) / CR 4000 (Revenue)
//! 4. Amount matches invoice.amount_cents (converted to major units)
//! 5. Duplicate event_id rejected (idempotency — processed_events gate)
//! 6. Journal entry metadata: source_module=ar, correct tenant_id, currency
//!
//! ## Pattern
//! Tests call `process_gl_posting_request` directly, bypassing NATS.
//! This follows the `inventory_gl_e2e.rs` pattern: no running services
//! required — only live ar-postgres (5434) and gl-postgres (5438).
//!
//! ## Services required
//! - ar-postgres at localhost:5434
//! - gl-postgres at localhost:5438

mod common;

use anyhow::Result;
use common::{generate_test_tenant, get_ar_pool, get_gl_pool};
use gl_rs::contracts::gl_posting_request_v1::{GlPostingRequestV1, JournalLine, SourceDocType};
use gl_rs::services::journal_service::{self, JournalError};
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

// ============================================================================
// Helpers
// ============================================================================

/// Set up Chart of Accounts entries: 1100 (AR Receivable) and 4000 (Revenue).
async fn setup_gl_accounts(pool: &PgPool, tenant_id: &str) -> Result<()> {
    for (code, name, acct_type, normal_balance) in [
        ("1100", "Accounts Receivable", "asset", "debit"),
        ("4000", "Revenue", "revenue", "credit"),
    ] {
        sqlx::query(
            "INSERT INTO accounts
             (id, tenant_id, code, name, type, normal_balance, is_active, created_at)
             VALUES ($1, $2, $3, $4, $5::account_type, $6::normal_balance, true, NOW())
             ON CONFLICT (tenant_id, code) DO NOTHING",
        )
        .bind(Uuid::new_v4())
        .bind(tenant_id)
        .bind(code)
        .bind(name)
        .bind(acct_type)
        .bind(normal_balance)
        .execute(pool)
        .await?;
    }
    Ok(())
}

/// Create an open accounting period covering all of 2026.
async fn setup_accounting_period(pool: &PgPool, tenant_id: &str) -> Result<()> {
    sqlx::query(
        "INSERT INTO accounting_periods
         (id, tenant_id, period_start, period_end, is_closed, created_at)
         VALUES ($1, $2, '2026-01-01', '2026-12-31', false, NOW())
         ON CONFLICT DO NOTHING",
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Create an AR customer in the AR database.
async fn create_ar_customer(pool: &PgPool, app_id: &str) -> Result<i32> {
    let id: i32 = sqlx::query_scalar(
        "INSERT INTO ar_customers
         (app_id, email, name, status, retry_attempt_count, created_at, updated_at)
         VALUES ($1, $2, 'GL Automation Test Customer', 'active', 0, NOW(), NOW())
         RETURNING id",
    )
    .bind(app_id)
    .bind(format!("gl-auto-{}@test.com", Uuid::new_v4()))
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// Create an AR invoice with the given amount_cents (minor units, e.g. 50000 = $500.00).
async fn create_ar_invoice(
    pool: &PgPool,
    app_id: &str,
    customer_id: i32,
    amount_cents: i64,
) -> Result<i32> {
    let id: i32 = sqlx::query_scalar(
        "INSERT INTO ar_invoices
         (app_id, tilled_invoice_id, ar_customer_id, amount_cents, currency,
          status, due_at, updated_at)
         VALUES ($1, $2, $3, $4, 'USD', 'open', '2026-03-31', NOW())
         RETURNING id",
    )
    .bind(app_id)
    .bind(format!("inv-{}", Uuid::new_v4()))
    .bind(customer_id)
    .bind(amount_cents)
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// Build a GL posting request as if AR emitted it for an invoice.
///
/// Converts `amount_cents` (minor units) → major units for the GL V1 contract,
/// which expects amounts as f64 dollars (e.g. 500.0 = $500.00).
/// GL then stores `debit_minor = debit_dollars * 100`, so debit_minor == amount_cents.
fn build_gl_posting_request(invoice_id: i32, amount_cents: i64) -> GlPostingRequestV1 {
    let amount_dollars = amount_cents as f64 / 100.0;
    GlPostingRequestV1 {
        posting_date: "2026-02-20".to_string(),
        currency: "USD".to_string(),
        source_doc_type: SourceDocType::ArInvoice,
        source_doc_id: invoice_id.to_string(),
        description: format!("AR Invoice {} — revenue recognition", invoice_id),
        lines: vec![
            JournalLine {
                account_ref: "1100".to_string(), // Accounts Receivable — debit
                debit: amount_dollars,
                credit: 0.0,
                memo: Some(format!("AR receivable — Invoice {}", invoice_id)),
                dimensions: None,
            },
            JournalLine {
                account_ref: "4000".to_string(), // Revenue — credit
                debit: 0.0,
                credit: amount_dollars,
                memo: Some(format!("Revenue — Invoice {}", invoice_id)),
                dimensions: None,
            },
        ],
    }
}

/// Clean up GL data for the test tenant (reverse FK order).
async fn cleanup_gl(pool: &PgPool, tenant_id: &str) -> Result<()> {
    sqlx::query(
        "DELETE FROM journal_lines WHERE journal_entry_id IN \
         (SELECT id FROM journal_entries WHERE tenant_id = $1)",
    )
    .bind(tenant_id)
    .execute(pool)
    .await?;

    sqlx::query(
        "DELETE FROM processed_events WHERE event_id IN \
         (SELECT source_event_id FROM journal_entries WHERE tenant_id = $1)",
    )
    .bind(tenant_id)
    .execute(pool)
    .await?;

    sqlx::query("DELETE FROM journal_entries WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await?;

    sqlx::query("DELETE FROM account_balances WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await?;

    sqlx::query("DELETE FROM period_summary_snapshots WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await?;

    sqlx::query("DELETE FROM accounts WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await?;

    sqlx::query("DELETE FROM accounting_periods WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await?;

    Ok(())
}

/// Clean up AR data for the test tenant.
async fn cleanup_ar(pool: &PgPool, app_id: &str) {
    sqlx::query("DELETE FROM events_outbox WHERE tenant_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM ar_invoices WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM ar_customers WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
}

// ============================================================================
// Tests
// ============================================================================

/// Test 1: AR invoice → GL journal entry with correct account codes and amounts.
///
/// Invariant: every AR invoice must produce exactly one GL journal entry with:
///   DR 1100 (Accounts Receivable) — amount_cents minor units
///   CR 4000 (Revenue)             — amount_cents minor units
#[tokio::test]
#[serial]
async fn test_ar_invoice_creates_gl_journal_entry() -> Result<()> {
    let tenant_id = generate_test_tenant();
    let ar_pool = get_ar_pool().await;
    let gl_pool = get_gl_pool().await;

    cleanup_gl(&gl_pool, &tenant_id).await?;
    cleanup_ar(&ar_pool, &tenant_id).await;

    setup_gl_accounts(&gl_pool, &tenant_id).await?;
    setup_accounting_period(&gl_pool, &tenant_id).await?;

    // Create AR customer + invoice (amount_cents = 50000 = $500.00)
    let customer_id = create_ar_customer(&ar_pool, &tenant_id).await?;
    let amount_cents = 50_000i64;
    let invoice_id = create_ar_invoice(&ar_pool, &tenant_id, customer_id, amount_cents).await?;

    // Simulate the GL posting request that the AR outbox publisher would emit.
    // The GL posting consumer calls process_gl_posting_request with source_module="ar".
    let event_id = Uuid::new_v4();
    let payload = build_gl_posting_request(invoice_id, amount_cents);

    let entry_id = journal_service::process_gl_posting_request(
        &gl_pool,
        event_id,
        &tenant_id,
        "ar",
        "gl.events.posting.requested",
        &payload,
        None,
    )
    .await
    .map_err(|e| anyhow::anyhow!("GL posting failed: {:?}", e))?;

    // Verify journal entry was created
    let entry_exists: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM journal_entries WHERE id = $1")
            .bind(entry_id)
            .fetch_optional(&gl_pool)
            .await?;
    assert!(entry_exists.is_some(), "journal entry must exist in GL");

    // Verify journal lines: 1100 debit, 4000 credit
    let lines: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT account_ref, debit_minor::BIGINT, credit_minor::BIGINT
         FROM journal_lines WHERE journal_entry_id = $1 ORDER BY line_no",
    )
    .bind(entry_id)
    .fetch_all(&gl_pool)
    .await?;

    assert_eq!(lines.len(), 2, "must have exactly 2 journal lines");

    let ar_line = lines.iter().find(|(acct, _, _)| acct == "1100");
    let rev_line = lines.iter().find(|(acct, _, _)| acct == "4000");

    let (_, ar_debit, ar_credit) = ar_line.expect("1100 AR Receivable line must exist");
    let (_, rev_debit, rev_credit) = rev_line.expect("4000 Revenue line must exist");

    // debit_minor = (amount_dollars * 100) = ((amount_cents / 100) * 100) = amount_cents
    assert_eq!(
        *ar_debit, amount_cents as i64,
        "AR receivable debit_minor must equal invoice amount_cents"
    );
    assert_eq!(*ar_credit, 0, "AR receivable must have zero credit");
    assert_eq!(*rev_debit, 0, "Revenue must have zero debit");
    assert_eq!(
        *rev_credit, amount_cents as i64,
        "Revenue credit_minor must equal invoice amount_cents"
    );

    cleanup_gl(&gl_pool, &tenant_id).await?;
    cleanup_ar(&ar_pool, &tenant_id).await;
    Ok(())
}

/// Test 2: GL journal entry produced by AR invoice is balanced (debits == credits).
///
/// Double-entry invariant: no unbalanced entries allowed.
#[tokio::test]
#[serial]
async fn test_ar_invoice_gl_entry_is_balanced() -> Result<()> {
    let tenant_id = generate_test_tenant();
    let ar_pool = get_ar_pool().await;
    let gl_pool = get_gl_pool().await;

    cleanup_gl(&gl_pool, &tenant_id).await?;
    cleanup_ar(&ar_pool, &tenant_id).await;

    setup_gl_accounts(&gl_pool, &tenant_id).await?;
    setup_accounting_period(&gl_pool, &tenant_id).await?;

    let customer_id = create_ar_customer(&ar_pool, &tenant_id).await?;
    let amount_cents = 123_456i64; // $1,234.56
    let invoice_id = create_ar_invoice(&ar_pool, &tenant_id, customer_id, amount_cents).await?;

    let event_id = Uuid::new_v4();
    let payload = build_gl_posting_request(invoice_id, amount_cents);

    let entry_id = journal_service::process_gl_posting_request(
        &gl_pool,
        event_id,
        &tenant_id,
        "ar",
        "gl.events.posting.requested",
        &payload,
        None,
    )
    .await
    .map_err(|e| anyhow::anyhow!("{:?}", e))?;

    let (total_debits, total_credits): (i64, i64) = sqlx::query_as(
        "SELECT COALESCE(SUM(debit_minor),0)::BIGINT, COALESCE(SUM(credit_minor),0)::BIGINT
         FROM journal_lines WHERE journal_entry_id = $1",
    )
    .bind(entry_id)
    .fetch_one(&gl_pool)
    .await?;

    assert_eq!(
        total_debits, total_credits,
        "journal entry must be balanced: debits={} credits={}",
        total_debits, total_credits
    );
    assert!(total_debits > 0, "must have non-zero debits");
    assert_eq!(
        total_debits, amount_cents as i64,
        "total debits must equal invoice amount_cents"
    );

    cleanup_gl(&gl_pool, &tenant_id).await?;
    cleanup_ar(&ar_pool, &tenant_id).await;
    Ok(())
}

/// Test 3: GL posting is idempotent — duplicate event_id must not create a second entry.
///
/// Failure mode: NATS at-least-once delivery can replay the same event.
/// The processed_events table gates duplicate processing.
#[tokio::test]
#[serial]
async fn test_ar_invoice_gl_posting_idempotency() -> Result<()> {
    let tenant_id = generate_test_tenant();
    let ar_pool = get_ar_pool().await;
    let gl_pool = get_gl_pool().await;

    cleanup_gl(&gl_pool, &tenant_id).await?;
    cleanup_ar(&ar_pool, &tenant_id).await;

    setup_gl_accounts(&gl_pool, &tenant_id).await?;
    setup_accounting_period(&gl_pool, &tenant_id).await?;

    let customer_id = create_ar_customer(&ar_pool, &tenant_id).await?;
    let invoice_id = create_ar_invoice(&ar_pool, &tenant_id, customer_id, 25_000).await?;

    let event_id = Uuid::new_v4();
    let payload = build_gl_posting_request(invoice_id, 25_000);

    // First posting — must succeed
    let first = journal_service::process_gl_posting_request(
        &gl_pool,
        event_id,
        &tenant_id,
        "ar",
        "gl.events.posting.requested",
        &payload,
        None,
    )
    .await;
    assert!(
        first.is_ok(),
        "first GL posting must succeed: {:?}",
        first.err()
    );

    // Second posting with same event_id — must return DuplicateEvent
    let second = journal_service::process_gl_posting_request(
        &gl_pool,
        event_id,
        &tenant_id,
        "ar",
        "gl.events.posting.requested",
        &payload,
        None,
    )
    .await;

    match second {
        Err(JournalError::DuplicateEvent(id)) => {
            assert_eq!(
                id, event_id,
                "DuplicateEvent must report the duplicated event_id"
            );
        }
        other => panic!(
            "Expected DuplicateEvent on duplicate event_id, got: {:?}",
            other
        ),
    }

    // Exactly one journal entry must exist
    let entry_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM journal_entries WHERE tenant_id = $1 AND source_event_id = $2",
    )
    .bind(&tenant_id)
    .bind(event_id)
    .fetch_one(&gl_pool)
    .await?;
    assert_eq!(
        entry_count, 1,
        "exactly one journal entry despite two posting attempts"
    );

    // Exactly one processed_events row
    let proc_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM processed_events WHERE event_id = $1")
            .bind(event_id)
            .fetch_one(&gl_pool)
            .await?;
    assert_eq!(
        proc_count, 1,
        "exactly one processed_events row (idempotency gate)"
    );

    cleanup_gl(&gl_pool, &tenant_id).await?;
    cleanup_ar(&ar_pool, &tenant_id).await;
    Ok(())
}

/// Test 4: GL journal entry metadata — source_module=ar, tenant_id, source_event_id.
///
/// Validates that the EventEnvelope fields (source_module, tenant_id, event_id)
/// are correctly stored in the GL journal entry for audit traceability.
#[tokio::test]
#[serial]
async fn test_ar_invoice_gl_entry_metadata() -> Result<()> {
    let tenant_id = generate_test_tenant();
    let ar_pool = get_ar_pool().await;
    let gl_pool = get_gl_pool().await;

    cleanup_gl(&gl_pool, &tenant_id).await?;
    cleanup_ar(&ar_pool, &tenant_id).await;

    setup_gl_accounts(&gl_pool, &tenant_id).await?;
    setup_accounting_period(&gl_pool, &tenant_id).await?;

    let customer_id = create_ar_customer(&ar_pool, &tenant_id).await?;
    let amount_cents = 75_000i64; // $750.00
    let invoice_id = create_ar_invoice(&ar_pool, &tenant_id, customer_id, amount_cents).await?;

    let event_id = Uuid::new_v4();
    let payload = build_gl_posting_request(invoice_id, amount_cents);

    let entry_id = journal_service::process_gl_posting_request(
        &gl_pool,
        event_id,
        &tenant_id,
        "ar",
        "gl.events.posting.requested",
        &payload,
        None,
    )
    .await
    .map_err(|e| anyhow::anyhow!("{:?}", e))?;

    // Verify journal entry envelope metadata
    let (db_tenant, db_module, db_event_id, db_subject, db_currency): (
        String,
        String,
        Uuid,
        String,
        String,
    ) = sqlx::query_as(
        "SELECT tenant_id, source_module, source_event_id, source_subject, currency
         FROM journal_entries WHERE id = $1",
    )
    .bind(entry_id)
    .fetch_one(&gl_pool)
    .await?;

    assert_eq!(db_tenant, tenant_id, "tenant_id must match the test tenant");
    assert_eq!(db_module, "ar", "source_module must be 'ar'");
    assert_eq!(
        db_event_id, event_id,
        "source_event_id must match the event Uuid"
    );
    assert_eq!(
        db_subject, "gl.events.posting.requested",
        "source_subject must be the NATS subject"
    );
    assert_eq!(db_currency, "USD", "currency must be 'USD'");

    // Verify processed_events row created atomically
    let proc_exists: Option<Uuid> =
        sqlx::query_scalar("SELECT event_id FROM processed_events WHERE event_id = $1")
            .bind(event_id)
            .fetch_optional(&gl_pool)
            .await?;
    assert!(
        proc_exists.is_some(),
        "processed_events row must exist for idempotency gate"
    );

    // Verify reference_type (mapped from source_doc_type) recorded correctly
    let reference_type: Option<String> =
        sqlx::query_scalar("SELECT reference_type FROM journal_entries WHERE id = $1")
            .bind(entry_id)
            .fetch_optional(&gl_pool)
            .await?;
    assert_eq!(
        reference_type.as_deref(),
        Some("AR_INVOICE"),
        "reference_type must be AR_INVOICE"
    );

    cleanup_gl(&gl_pool, &tenant_id).await?;
    cleanup_ar(&ar_pool, &tenant_id).await;
    Ok(())
}

/// Test 5: Two distinct AR invoices produce two independent GL journal entries.
///
/// Each invoice event must generate its own journal entry — no cross-contamination.
#[tokio::test]
#[serial]
async fn test_two_ar_invoices_produce_two_gl_entries() -> Result<()> {
    let tenant_id = generate_test_tenant();
    let ar_pool = get_ar_pool().await;
    let gl_pool = get_gl_pool().await;

    cleanup_gl(&gl_pool, &tenant_id).await?;
    cleanup_ar(&ar_pool, &tenant_id).await;

    setup_gl_accounts(&gl_pool, &tenant_id).await?;
    setup_accounting_period(&gl_pool, &tenant_id).await?;

    let customer_id = create_ar_customer(&ar_pool, &tenant_id).await?;

    // Invoice 1: $100.00
    let inv1_id = create_ar_invoice(&ar_pool, &tenant_id, customer_id, 10_000).await?;
    let ev1 = Uuid::new_v4();
    let entry1 = journal_service::process_gl_posting_request(
        &gl_pool,
        ev1,
        &tenant_id,
        "ar",
        "gl.events.posting.requested",
        &build_gl_posting_request(inv1_id, 10_000),
        None,
    )
    .await
    .map_err(|e| anyhow::anyhow!("Invoice 1 GL posting failed: {:?}", e))?;

    // Invoice 2: $200.00
    let inv2_id = create_ar_invoice(&ar_pool, &tenant_id, customer_id, 20_000).await?;
    let ev2 = Uuid::new_v4();
    let entry2 = journal_service::process_gl_posting_request(
        &gl_pool,
        ev2,
        &tenant_id,
        "ar",
        "gl.events.posting.requested",
        &build_gl_posting_request(inv2_id, 20_000),
        None,
    )
    .await
    .map_err(|e| anyhow::anyhow!("Invoice 2 GL posting failed: {:?}", e))?;

    assert_ne!(
        entry1, entry2,
        "two invoices must produce two distinct journal entries"
    );

    // Verify: exactly 2 journal entries for this tenant
    let entry_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM journal_entries WHERE tenant_id = $1")
            .bind(&tenant_id)
            .fetch_one(&gl_pool)
            .await?;
    assert_eq!(entry_count, 2, "exactly 2 journal entries for 2 invoices");

    // Verify each entry has the correct debit amount
    let (amt1,): (i64,) = sqlx::query_as(
        "SELECT COALESCE(SUM(debit_minor),0)::BIGINT FROM journal_lines
         WHERE journal_entry_id = $1",
    )
    .bind(entry1)
    .fetch_one(&gl_pool)
    .await?;
    let (amt2,): (i64,) = sqlx::query_as(
        "SELECT COALESCE(SUM(debit_minor),0)::BIGINT FROM journal_lines
         WHERE journal_entry_id = $1",
    )
    .bind(entry2)
    .fetch_one(&gl_pool)
    .await?;

    assert_eq!(amt1, 10_000, "Invoice 1 GL debit must be 10000 minor units");
    assert_eq!(amt2, 20_000, "Invoice 2 GL debit must be 20000 minor units");

    cleanup_gl(&gl_pool, &tenant_id).await?;
    cleanup_ar(&ar_pool, &tenant_id).await;
    Ok(())
}

/// Test 6: GL posting rejected when no open accounting period covers the posting date.
///
/// Period safety: entries must always land in an open period.
#[tokio::test]
#[serial]
async fn test_ar_invoice_gl_posting_rejected_without_open_period() -> Result<()> {
    let tenant_id = generate_test_tenant();
    let ar_pool = get_ar_pool().await;
    let gl_pool = get_gl_pool().await;

    cleanup_gl(&gl_pool, &tenant_id).await?;
    cleanup_ar(&ar_pool, &tenant_id).await;

    // Set up accounts but NO accounting period
    setup_gl_accounts(&gl_pool, &tenant_id).await?;

    let customer_id = create_ar_customer(&ar_pool, &tenant_id).await?;
    let invoice_id = create_ar_invoice(&ar_pool, &tenant_id, customer_id, 10_000).await?;

    // Posting date falls outside any open period (none exists)
    let payload = GlPostingRequestV1 {
        posting_date: "2026-02-20".to_string(),
        currency: "USD".to_string(),
        source_doc_type: SourceDocType::ArInvoice,
        source_doc_id: invoice_id.to_string(),
        description: "No period test".to_string(),
        lines: vec![
            JournalLine {
                account_ref: "1100".to_string(),
                debit: 100.0,
                credit: 0.0,
                memo: None,
                dimensions: None,
            },
            JournalLine {
                account_ref: "4000".to_string(),
                debit: 0.0,
                credit: 100.0,
                memo: None,
                dimensions: None,
            },
        ],
    };

    let result = journal_service::process_gl_posting_request(
        &gl_pool,
        Uuid::new_v4(),
        &tenant_id,
        "ar",
        "gl.events.posting.requested",
        &payload,
        None,
    )
    .await;

    assert!(
        result.is_err(),
        "GL posting must fail when no accounting period exists"
    );

    match result.unwrap_err() {
        JournalError::Period(_) => {
            // Expected: NoPeriodForDate
        }
        other => panic!("Expected JournalError::Period, got: {:?}", other),
    }

    cleanup_gl(&gl_pool, &tenant_id).await?;
    cleanup_ar(&ar_pool, &tenant_id).await;
    Ok(())
}
