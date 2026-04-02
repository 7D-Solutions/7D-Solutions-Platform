//! E2E Test: AR Credit Note Issuance (bd-1gt)
//!
//! **Coverage:**
//! 1. Issue a credit note against a finalized invoice — row inserted + outbox event emitted atomically
//! 2. Idempotency: duplicate credit_note_id is a no-op (no second row, no second event)
//! 3. Invalid amount (≤0) rejected before DB touch
//! 4. Unknown invoice rejected with InvoiceNotFound
//! 5. Outbox atomicity: credit note row and outbox event are in the same transaction
//!
//! See ar_credit_note_balance_e2e.rs for bd-35dm tests (balance, NATS, over-credit guard).
//!
//! **Pattern:** No Docker, no mocks — uses live AR database pool via common::get_ar_pool()

mod common;

use anyhow::Result;
use ar_rs::credit_notes::{issue_credit_note, IssueCreditNoteRequest, IssueCreditNoteResult};
use common::{generate_test_tenant, get_ar_pool};
use uuid::Uuid;

// ============================================================================
// Test helpers
// ============================================================================

/// Insert a minimal customer and invoice for test isolation.
/// Returns (customer_id, invoice_internal_id, tilled_invoice_id).
async fn create_test_invoice(
    pool: &sqlx::PgPool,
    tenant_id: &str,
    amount_cents: i64,
) -> Result<(i32, i32)> {
    let customer_id: i32 = sqlx::query_scalar(
        r#"
        INSERT INTO ar_customers (app_id, email, name, status, retry_attempt_count, created_at, updated_at)
        VALUES ($1, $2, $3, 'active', 0, NOW(), NOW())
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(format!("cn-test-{}@test.local", Uuid::new_v4()))
    .bind("CN Test Customer")
    .fetch_one(pool)
    .await?;

    let invoice_id: i32 = sqlx::query_scalar(
        r#"
        INSERT INTO ar_invoices (
            app_id, tilled_invoice_id, ar_customer_id, status, amount_cents, currency,
            created_at, updated_at
        )
        VALUES ($1, $2, $3, 'open', $4, 'usd', NOW(), NOW())
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(format!("in_cn_{}", Uuid::new_v4()))
    .bind(customer_id)
    .bind(amount_cents)
    .fetch_one(pool)
    .await?;

    Ok((customer_id, invoice_id))
}

/// Count outbox events for a given aggregate (credit_note) id.
async fn count_outbox_events(pool: &sqlx::PgPool, credit_note_id: &str) -> Result<i64> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox WHERE aggregate_type = 'credit_note' AND aggregate_id = $1",
    )
    .bind(credit_note_id)
    .fetch_one(pool)
    .await?;
    Ok(count)
}

/// Cleanup tenant data from AR tables (in reverse FK order).
async fn cleanup_tenant(pool: &sqlx::PgPool, tenant_id: &str) -> Result<()> {
    sqlx::query("DELETE FROM events_outbox WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM ar_credit_notes WHERE app_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM ar_invoice_attempts WHERE app_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM ar_invoices WHERE app_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM ar_customers WHERE app_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await?;
    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

/// Test: issuing a credit note creates a row + outbox event atomically.
#[tokio::test]
async fn test_credit_note_issued_with_outbox_event() {
    let pool = get_ar_pool().await;
    let tenant_id = generate_test_tenant();
    let credit_note_id = Uuid::new_v4();

    let (_customer_id, invoice_id) = create_test_invoice(&pool, &tenant_id, 10000)
        .await
        .expect("failed to create test invoice");

    let req = IssueCreditNoteRequest {
        credit_note_id,
        app_id: tenant_id.clone(),
        customer_id: format!("cust-{}", tenant_id),
        invoice_id,
        amount_minor: 5000,
        currency: "usd".to_string(),
        reason: "billing_error".to_string(),
        reference_id: None,
        issued_by: Some("test-suite".to_string()),
        correlation_id: Uuid::new_v4().to_string(),
        causation_id: None,
    };

    let result = issue_credit_note(&pool, req)
        .await
        .expect("issue_credit_note failed");

    match result {
        IssueCreditNoteResult::Issued {
            credit_note_row_id,
            credit_note_id: returned_id,
            ..
        } => {
            assert_eq!(returned_id, credit_note_id);
            assert!(credit_note_row_id > 0);

            // Verify outbox event was written atomically
            let event_count = count_outbox_events(&pool, &credit_note_id.to_string())
                .await
                .expect("failed to count outbox events");
            assert_eq!(event_count, 1, "expected exactly 1 outbox event");

            // Verify the credit_note_issued event type is correct
            let event_type: String = sqlx::query_scalar(
                "SELECT event_type FROM events_outbox WHERE aggregate_type = 'credit_note' AND aggregate_id = $1",
            )
            .bind(credit_note_id.to_string())
            .fetch_one(&pool)
            .await
            .expect("failed to fetch event type");
            assert_eq!(event_type, "ar.credit_note_issued");
        }
        IssueCreditNoteResult::AlreadyProcessed { .. } => {
            panic!("expected Issued, got AlreadyProcessed");
        }
    }

    cleanup_tenant(&pool, &tenant_id).await.unwrap();
}

/// Test: duplicate credit_note_id is a no-op (idempotency).
#[tokio::test]
async fn test_credit_note_idempotency() {
    let pool = get_ar_pool().await;
    let tenant_id = generate_test_tenant();
    let credit_note_id = Uuid::new_v4();

    let (_customer_id, invoice_id) = create_test_invoice(&pool, &tenant_id, 10000)
        .await
        .expect("failed to create test invoice");

    let make_req = || IssueCreditNoteRequest {
        credit_note_id,
        app_id: tenant_id.clone(),
        customer_id: format!("cust-{}", tenant_id),
        invoice_id,
        amount_minor: 2500,
        currency: "usd".to_string(),
        reason: "service_credit".to_string(),
        reference_id: None,
        issued_by: None,
        correlation_id: Uuid::new_v4().to_string(),
        causation_id: None,
    };

    // First issue: should succeed
    let first = issue_credit_note(&pool, make_req())
        .await
        .expect("first issue failed");
    assert!(
        matches!(first, IssueCreditNoteResult::Issued { .. }),
        "expected Issued on first call"
    );

    // Second issue with same credit_note_id: should be AlreadyProcessed
    let second = issue_credit_note(&pool, make_req())
        .await
        .expect("second issue failed");
    assert!(
        matches!(second, IssueCreditNoteResult::AlreadyProcessed { .. }),
        "expected AlreadyProcessed on second call"
    );

    // Only one credit note row should exist
    let row_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM ar_credit_notes WHERE credit_note_id = $1")
            .bind(credit_note_id)
            .fetch_one(&pool)
            .await
            .expect("failed to count credit note rows");
    assert_eq!(row_count, 1, "expected exactly 1 credit note row");

    // Only one outbox event should exist
    let event_count = count_outbox_events(&pool, &credit_note_id.to_string())
        .await
        .expect("failed to count outbox events");
    assert_eq!(
        event_count, 1,
        "expected exactly 1 outbox event (no duplicate)"
    );

    cleanup_tenant(&pool, &tenant_id).await.unwrap();
}

/// Test: zero or negative amount is rejected before any DB operation.
#[tokio::test]
async fn test_credit_note_invalid_amount_rejected() {
    let pool = get_ar_pool().await;
    let tenant_id = generate_test_tenant();

    let (_customer_id, invoice_id) = create_test_invoice(&pool, &tenant_id, 10000)
        .await
        .expect("failed to create test invoice");

    // amount_minor = 0 should be rejected
    let req = IssueCreditNoteRequest {
        credit_note_id: Uuid::new_v4(),
        app_id: tenant_id.clone(),
        customer_id: "cust-x".to_string(),
        invoice_id,
        amount_minor: 0,
        currency: "usd".to_string(),
        reason: "test".to_string(),
        reference_id: None,
        issued_by: None,
        correlation_id: Uuid::new_v4().to_string(),
        causation_id: None,
    };

    let err = issue_credit_note(&pool, req)
        .await
        .expect_err("expected InvalidAmount error");
    assert!(
        matches!(err, ar_rs::credit_notes::CreditNoteError::InvalidAmount(0)),
        "expected InvalidAmount(0), got {:?}",
        err
    );

    cleanup_tenant(&pool, &tenant_id).await.unwrap();
}

/// Test: unknown invoice_id is rejected with InvoiceNotFound.
#[tokio::test]
async fn test_credit_note_unknown_invoice_rejected() {
    let pool = get_ar_pool().await;
    let tenant_id = generate_test_tenant();

    let req = IssueCreditNoteRequest {
        credit_note_id: Uuid::new_v4(),
        app_id: tenant_id.clone(),
        customer_id: "cust-x".to_string(),
        invoice_id: i32::MAX, // very unlikely to exist
        amount_minor: 1000,
        currency: "usd".to_string(),
        reason: "test".to_string(),
        reference_id: None,
        issued_by: None,
        correlation_id: Uuid::new_v4().to_string(),
        causation_id: None,
    };

    let err = issue_credit_note(&pool, req)
        .await
        .expect_err("expected InvoiceNotFound error");
    assert!(
        matches!(
            err,
            ar_rs::credit_notes::CreditNoteError::InvoiceNotFound { .. }
        ),
        "expected InvoiceNotFound, got {:?}",
        err
    );

    // No credit note rows should have been inserted
    let row_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM ar_credit_notes WHERE app_id = $1")
            .bind(&tenant_id)
            .fetch_one(&pool)
            .await
            .expect("count failed");
    assert_eq!(
        row_count, 0,
        "no credit note rows should exist after rejection"
    );

    cleanup_tenant(&pool, &tenant_id).await.unwrap();
}

/// Test: credit note row and outbox event are committed atomically
/// (both present or both absent — no split-brain).
#[tokio::test]
async fn test_credit_note_outbox_atomicity() {
    let pool = get_ar_pool().await;
    let tenant_id = generate_test_tenant();
    let credit_note_id = Uuid::new_v4();

    let (_customer_id, invoice_id) = create_test_invoice(&pool, &tenant_id, 20000)
        .await
        .expect("failed to create test invoice");

    let req = IssueCreditNoteRequest {
        credit_note_id,
        app_id: tenant_id.clone(),
        customer_id: format!("cust-{}", tenant_id),
        invoice_id,
        amount_minor: 10000,
        currency: "usd".to_string(),
        reason: "dispute_settled".to_string(),
        reference_id: Some("dispute-ref-001".to_string()),
        issued_by: Some("ops-team".to_string()),
        correlation_id: Uuid::new_v4().to_string(),
        causation_id: Some("dispute-001".to_string()),
    };

    issue_credit_note(&pool, req)
        .await
        .expect("issue_credit_note failed");

    // Both the credit note row and the outbox event must be present
    let note_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM ar_credit_notes WHERE credit_note_id = $1")
            .bind(credit_note_id)
            .fetch_one(&pool)
            .await
            .expect("count failed");

    let event_count = count_outbox_events(&pool, &credit_note_id.to_string())
        .await
        .expect("count failed");

    assert_eq!(note_count, 1, "credit note row must exist after issue");
    assert_eq!(
        event_count, 1,
        "outbox event must exist atomically with credit note row"
    );

    // Verify outbox event has correct mutation_class = DATA_MUTATION
    let mutation_class: String =
        sqlx::query_scalar("SELECT mutation_class FROM events_outbox WHERE aggregate_id = $1")
            .bind(credit_note_id.to_string())
            .fetch_one(&pool)
            .await
            .expect("fetch mutation_class failed");
    assert_eq!(mutation_class, "DATA_MUTATION");

    cleanup_tenant(&pool, &tenant_id).await.unwrap();
}
