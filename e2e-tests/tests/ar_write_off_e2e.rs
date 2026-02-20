//! E2E Test: AR Write-off — mark invoice uncollectible, verify GL write-off event (bd-cxyg)
//!
//! **Coverage:**
//! 1. Write off an overdue invoice → write-off row + REVERSAL outbox event + GL journal entry
//! 2. NATS event fires with correct written_off_amount_minor and tenant_id
//! 3. GL journal entry: BAD_DEBT debit + AR receivable credit (balanced, equal to write-off amount)
//! 4. Second write-off attempt on same invoice → AlreadyWrittenOff (422 equivalent)
//!
//! ## Chain tested
//! write_off_invoice → outbox (REVERSAL) → NATS publish → GL write-off consumer → journal entry
//!
//! ## Pattern
//! No Docker, no mocks. Real AR-postgres (5434) and GL-postgres (5438) + NATS.
//! write_off_invoice called directly → outbox event published manually to NATS →
//! NATS subscriber receives and verifies payload → process_writeoff_posting posts GL entry.
//!
//! ## Services required
//! - ar-postgres at localhost:5434
//! - gl-postgres at localhost:5438
//! - NATS at localhost:4222

mod common;

use anyhow::Result;
use ar_rs::{
    events::outbox::{mark_as_published},
    write_offs::{write_off_invoice, WriteOffInvoiceRequest, WriteOffInvoiceResult},
};
use common::{generate_test_tenant, get_ar_pool, get_gl_pool, setup_nats_client, subscribe_to_events};
use futures::StreamExt;
use gl_rs::consumer::gl_writeoff_consumer::{process_writeoff_posting, InvoiceWrittenOffPayload};
use serial_test::serial;
use sqlx::PgPool;
use std::time::Duration;
use uuid::Uuid;

// ============================================================================
// Helpers
// ============================================================================

/// Insert GL accounts BAD_DEBT + AR for the test tenant.
async fn setup_gl_accounts(pool: &PgPool, tenant_id: &str) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO accounts (id, tenant_id, code, name, type, normal_balance, is_active)
        VALUES
            (gen_random_uuid(), $1, 'BAD_DEBT', 'Bad Debt Expense', 'expense', 'debit', true),
            (gen_random_uuid(), $1, 'AR', 'Accounts Receivable', 'asset', 'debit', true)
        ON CONFLICT (tenant_id, code) DO NOTHING
        "#,
    )
    .bind(tenant_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Create an open accounting period covering all of 2026.
async fn setup_accounting_period(pool: &PgPool, tenant_id: &str) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO accounting_periods (tenant_id, period_start, period_end, is_closed)
        VALUES ($1, '2026-01-01', '2026-12-31', false)
        "#,
    )
    .bind(tenant_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Create AR customer + overdue invoice (due_at backdated by `days_ago` days).
/// Returns (customer_id, invoice_id).
async fn create_overdue_invoice(
    pool: &PgPool,
    tenant_id: &str,
    amount_cents: i32,
    days_ago: u32,
) -> Result<(i32, i32)> {
    let customer_id: i32 = sqlx::query_scalar(
        "INSERT INTO ar_customers (app_id, email, name, status, retry_attempt_count, created_at, updated_at)
         VALUES ($1, $2, 'WriteOff E2E Customer', 'active', 0, NOW(), NOW())
         RETURNING id",
    )
    .bind(tenant_id)
    .bind(format!("wo-e2e-{}@test.local", Uuid::new_v4()))
    .fetch_one(pool)
    .await?;

    let invoice_id: i32 = sqlx::query_scalar(&format!(
        "INSERT INTO ar_invoices (app_id, tilled_invoice_id, ar_customer_id, status,
             amount_cents, currency, due_at, created_at, updated_at)
         VALUES ($1, $2, $3, 'open', $4, 'usd', NOW() - INTERVAL '{days_ago} days', NOW(), NOW())
         RETURNING id",
        days_ago = days_ago
    ))
    .bind(tenant_id)
    .bind(format!("inv-wo-e2e-{}", Uuid::new_v4()))
    .bind(customer_id)
    .bind(amount_cents)
    .fetch_one(pool)
    .await?;

    Ok((customer_id, invoice_id))
}

/// Cleanup AR test data for a tenant (reverse FK order).
async fn cleanup_ar(pool: &PgPool, tenant_id: &str) {
    for sql in [
        "DELETE FROM events_outbox WHERE tenant_id = $1",
        "DELETE FROM ar_invoice_write_offs WHERE app_id = $1",
        "DELETE FROM ar_invoice_attempts WHERE app_id = $1",
        "DELETE FROM ar_invoices WHERE app_id = $1",
        "DELETE FROM ar_customers WHERE app_id = $1",
    ] {
        sqlx::query(sql).bind(tenant_id).execute(pool).await.ok();
    }
}

/// Cleanup GL test data for a tenant (reverse FK order).
async fn cleanup_gl(pool: &PgPool, tenant_id: &str) {
    for sql in [
        "DELETE FROM journal_lines WHERE journal_entry_id IN \
             (SELECT id FROM journal_entries WHERE tenant_id = $1)",
        "DELETE FROM processed_events WHERE event_id IN \
             (SELECT source_event_id FROM journal_entries WHERE tenant_id = $1)",
        "DELETE FROM journal_entries WHERE tenant_id = $1",
        "DELETE FROM account_balances WHERE tenant_id = $1",
        "DELETE FROM period_summary_snapshots WHERE tenant_id = $1",
        "DELETE FROM accounts WHERE tenant_id = $1",
        "DELETE FROM accounting_periods WHERE tenant_id = $1",
    ] {
        sqlx::query(sql).bind(tenant_id).execute(pool).await.ok();
    }
}

// ============================================================================
// Tests
// ============================================================================

/// Test 1: Full write-off chain — NATS event fires with correct payload, GL journal created.
///
/// Invariants:
/// - Write-off creates a REVERSAL outbox event (mutation_class = REVERSAL)
/// - NATS message carries correct written_off_amount_minor and tenant_id
/// - GL: BAD_DEBT debited, AR credited, journal entry balanced
#[tokio::test]
#[serial]
async fn test_write_off_nats_event_and_gl_entry() -> Result<()> {
    let ar_pool = get_ar_pool().await;
    let gl_pool = get_gl_pool().await;
    let tenant_id = generate_test_tenant();
    const AMOUNT: i32 = 50_000; // $500.00

    cleanup_ar(&ar_pool, &tenant_id).await;
    cleanup_gl(&gl_pool, &tenant_id).await;
    setup_gl_accounts(&gl_pool, &tenant_id).await?;
    setup_accounting_period(&gl_pool, &tenant_id).await?;

    // Create overdue invoice (45 days past due)
    let (_customer_id, invoice_id) =
        create_overdue_invoice(&ar_pool, &tenant_id, AMOUNT, 45).await?;

    // Subscribe to NATS before issuing write-off — subject follows ar.events.{event_type}
    let nats = setup_nats_client().await;
    let nats_subject = "ar.events.ar.invoice_written_off";
    let mut sub = subscribe_to_events(&nats, nats_subject).await;

    // Issue write-off
    let write_off_id = Uuid::new_v4();
    let req = WriteOffInvoiceRequest {
        write_off_id,
        app_id: tenant_id.clone(),
        invoice_id,
        customer_id: format!("cust-{}", &tenant_id[..8]),
        written_off_amount_minor: AMOUNT as i64,
        currency: "usd".to_string(),
        reason: "uncollectible".to_string(),
        authorized_by: Some("finance@test.local".to_string()),
        correlation_id: Uuid::new_v4().to_string(),
        causation_id: None,
    };

    let result = write_off_invoice(&ar_pool, req)
        .await
        .expect("write_off_invoice must succeed");

    let written_off_at = match result {
        WriteOffInvoiceResult::WrittenOff {
            write_off_row_id,
            write_off_id: returned_id,
            written_off_at,
        } => {
            assert_eq!(returned_id, write_off_id, "write_off_id must round-trip");
            assert!(write_off_row_id > 0, "write_off_row_id must be a positive DB serial");
            written_off_at
        }
        other => panic!("expected WrittenOff, got {:?}", other),
    };

    // Verify outbox event: type = ar.invoice_written_off, mutation_class = REVERSAL
    let (outbox_event_id, outbox_payload): (Uuid, serde_json::Value) = sqlx::query_as(
        "SELECT event_id, payload FROM events_outbox
         WHERE tenant_id = $1 AND event_type = 'ar.invoice_written_off'
         LIMIT 1",
    )
    .bind(&tenant_id)
    .fetch_one(&ar_pool)
    .await
    .expect("outbox event must exist after write-off");

    let mutation_class: String = sqlx::query_scalar(
        "SELECT mutation_class FROM events_outbox WHERE event_id = $1",
    )
    .bind(outbox_event_id)
    .fetch_one(&ar_pool)
    .await
    .expect("fetch mutation_class from outbox");

    assert_eq!(mutation_class, "REVERSAL", "write-off outbox event must be REVERSAL class");

    // Manually publish outbox event to NATS (simulates the background publisher)
    let payload_bytes = serde_json::to_vec(&outbox_payload).expect("serialize outbox payload");
    nats.publish(nats_subject.to_string(), payload_bytes.into())
        .await
        .expect("NATS publish must succeed");
    mark_as_published(&ar_pool, outbox_event_id)
        .await
        .expect("mark_as_published must succeed");

    // Receive event from NATS and verify payload fields
    let msg = tokio::time::timeout(Duration::from_secs(5), sub.next())
        .await
        .expect("timeout: NATS message not received within 5s")
        .expect("NATS subscriber must not be closed");

    let envelope: serde_json::Value =
        serde_json::from_slice(&msg.payload).expect("deserialize NATS message");

    assert_eq!(
        envelope["tenant_id"].as_str().unwrap_or(""),
        tenant_id,
        "NATS envelope tenant_id must match the write-off tenant"
    );

    let payload_inner = &envelope["payload"];
    assert_eq!(
        payload_inner["written_off_amount_minor"].as_i64().unwrap_or(0),
        AMOUNT as i64,
        "NATS payload written_off_amount_minor must equal {}",
        AMOUNT
    );

    // Process GL write-off posting using the outbox event_id as idempotency key
    let gl_payload = InvoiceWrittenOffPayload {
        tenant_id: tenant_id.clone(),
        invoice_id: invoice_id.to_string(),
        customer_id: format!("cust-{}", &tenant_id[..8]),
        written_off_amount_minor: AMOUNT as i64,
        currency: "usd".to_string(),
        reason: "uncollectible".to_string(),
        authorized_by: Some("finance@test.local".to_string()),
        written_off_at,
    };

    let entry_id =
        process_writeoff_posting(&gl_pool, outbox_event_id, &tenant_id, "ar", &gl_payload)
            .await
            .expect("GL write-off posting must succeed");

    // Verify journal lines: BAD_DEBT debit, AR credit
    let lines: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT account_ref,
                COALESCE(debit_minor, 0)::BIGINT,
                COALESCE(credit_minor, 0)::BIGINT
         FROM journal_lines
         WHERE journal_entry_id = $1
         ORDER BY line_no",
    )
    .bind(entry_id)
    .fetch_all(&gl_pool)
    .await?;

    assert_eq!(lines.len(), 2, "exactly 2 journal lines (BAD_DEBT DR + AR CR)");

    let bad_debt_line = lines
        .iter()
        .find(|(acct, _, _)| acct == "BAD_DEBT")
        .expect("BAD_DEBT line must exist");
    let ar_line = lines
        .iter()
        .find(|(acct, _, _)| acct == "AR")
        .expect("AR line must exist");

    let expected_minor = AMOUNT as i64; // 50000 minor units = $500.00

    assert_eq!(
        bad_debt_line.1, expected_minor,
        "BAD_DEBT debit_minor must equal write-off amount"
    );
    assert_eq!(bad_debt_line.2, 0, "BAD_DEBT credit must be 0");
    assert_eq!(ar_line.1, 0, "AR debit must be 0");
    assert_eq!(
        ar_line.2, expected_minor,
        "AR credit_minor must equal write-off amount"
    );

    // Verify balance invariant: debits == credits
    let total_debit: i64 = lines.iter().map(|(_, d, _)| d).sum();
    let total_credit: i64 = lines.iter().map(|(_, _, c)| c).sum();
    assert_eq!(
        total_debit, total_credit,
        "journal entry must be balanced: debit={} credit={}",
        total_debit, total_credit
    );
    assert!(total_debit > 0, "journal entry must have non-zero debits");

    println!(
        "✅ PASS: write-off chain complete — invoice {} written off, NATS event fired, \
         GL entry {} created (BAD_DEBT DR ${:.2} / AR CR ${:.2})",
        invoice_id,
        entry_id,
        expected_minor as f64 / 100.0,
        expected_minor as f64 / 100.0,
    );

    cleanup_ar(&ar_pool, &tenant_id).await;
    cleanup_gl(&gl_pool, &tenant_id).await;
    Ok(())
}

/// Test 2: Second write-off on same invoice → AlreadyWrittenOff (422 equivalent).
///
/// Invariant: exactly one write-off per invoice. A second attempt with a different
/// write_off_id must be rejected — not silently ignored.
/// This maps to HTTP 422 (Unprocessable Entity) / CONFLICT at the API layer.
#[tokio::test]
#[serial]
async fn test_second_write_off_returns_already_written_off() -> Result<()> {
    let ar_pool = get_ar_pool().await;
    let tenant_id = generate_test_tenant();

    cleanup_ar(&ar_pool, &tenant_id).await;

    // Create overdue invoice (60 days past due)
    let (_customer_id, invoice_id) =
        create_overdue_invoice(&ar_pool, &tenant_id, 30_000, 60).await?;

    // First write-off — must succeed
    let first_req = WriteOffInvoiceRequest {
        write_off_id: Uuid::new_v4(),
        app_id: tenant_id.clone(),
        invoice_id,
        customer_id: format!("cust-{}", &tenant_id[..8]),
        written_off_amount_minor: 30_000,
        currency: "usd".to_string(),
        reason: "uncollectible".to_string(),
        authorized_by: None,
        correlation_id: Uuid::new_v4().to_string(),
        causation_id: None,
    };

    let first_result = write_off_invoice(&ar_pool, first_req)
        .await
        .expect("first write-off must not error");
    assert!(
        matches!(first_result, WriteOffInvoiceResult::WrittenOff { .. }),
        "first write-off must return WrittenOff, got {:?}",
        first_result
    );

    // Second write-off on same invoice (different write_off_id) — must return AlreadyWrittenOff
    let second_req = WriteOffInvoiceRequest {
        write_off_id: Uuid::new_v4(), // different ID — not an idempotency replay
        app_id: tenant_id.clone(),
        invoice_id,
        customer_id: format!("cust-{}", &tenant_id[..8]),
        written_off_amount_minor: 30_000,
        currency: "usd".to_string(),
        reason: "duplicate_attempt".to_string(),
        authorized_by: None,
        correlation_id: Uuid::new_v4().to_string(),
        causation_id: None,
    };

    let second_result = write_off_invoice(&ar_pool, second_req)
        .await
        .expect("second write-off call must not hard-error");

    match second_result {
        WriteOffInvoiceResult::AlreadyWrittenOff {
            invoice_id: rejected_id,
        } => {
            assert_eq!(
                rejected_id, invoice_id,
                "AlreadyWrittenOff must report the correct invoice_id"
            );
        }
        other => panic!(
            "expected AlreadyWrittenOff for invoice {}, got {:?}",
            invoice_id, other
        ),
    }

    // Exactly one write-off row must exist for this invoice
    let row_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM ar_invoice_write_offs WHERE invoice_id = $1")
            .bind(invoice_id)
            .fetch_one(&ar_pool)
            .await?;
    assert_eq!(row_count, 1, "exactly one write-off row per invoice (guard enforced)");

    // Exactly one outbox event must exist (rejected attempt produces no event)
    let event_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox
         WHERE tenant_id = $1 AND event_type = 'ar.invoice_written_off'",
    )
    .bind(&tenant_id)
    .fetch_one(&ar_pool)
    .await?;
    assert_eq!(
        event_count, 1,
        "exactly one write-off outbox event (no duplicate from rejected attempt)"
    );

    println!(
        "✅ PASS: double write-off guard — invoice {} correctly rejected AlreadyWrittenOff \
         (HTTP 422/CONFLICT equivalent)",
        invoice_id
    );

    cleanup_ar(&ar_pool, &tenant_id).await;
    Ok(())
}
