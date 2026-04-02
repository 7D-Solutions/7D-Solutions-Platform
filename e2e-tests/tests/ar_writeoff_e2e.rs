//! E2E Test: AR Invoice Write-off / Bad Debt (bd-2f2)
//!
//! **Coverage:**
//! 1. Write off an invoice — row inserted + outbox REVERSAL event emitted atomically
//! 2. Idempotency: duplicate write_off_id is a no-op (no second row, no second event)
//! 3. Double write-off on same invoice returns AlreadyWrittenOff (unique constraint)
//! 4. Invalid amount (≤0) rejected before DB touch
//! 5. Unknown invoice rejected with InvoiceNotFound
//! 6. Outbox atomicity: write-off row and outbox event in same transaction
//! 7. Outbox event has mutation_class = REVERSAL (not DATA_MUTATION)
//!
//! **Pattern:** No Docker, no mocks — uses live AR database pool via common::get_ar_pool()

mod common;

use anyhow::Result;
use ar_rs::write_offs::{write_off_invoice, WriteOffInvoiceRequest, WriteOffInvoiceResult};
use common::{generate_test_tenant, get_ar_pool};
use uuid::Uuid;

// ============================================================================
// Test helpers
// ============================================================================

/// Insert a minimal customer and invoice for test isolation.
/// Returns (customer_id, invoice_internal_id).
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
    .bind(format!("wo-test-{}@test.local", Uuid::new_v4()))
    .bind("WO Test Customer")
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
    .bind(format!("in_wo_{}", Uuid::new_v4()))
    .bind(customer_id)
    .bind(amount_cents)
    .fetch_one(pool)
    .await?;

    Ok((customer_id, invoice_id))
}

/// Count outbox events for a given aggregate (invoice_write_off) id.
async fn count_outbox_events(pool: &sqlx::PgPool, write_off_id: &str) -> Result<i64> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox WHERE aggregate_type = 'invoice_write_off' AND aggregate_id = $1",
    )
    .bind(write_off_id)
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
    sqlx::query("DELETE FROM ar_invoice_write_offs WHERE app_id = $1")
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

/// Test: writing off an invoice creates a row + REVERSAL outbox event atomically.
#[tokio::test]
async fn test_write_off_invoice_with_reversal_outbox_event() {
    let pool = get_ar_pool().await;
    let tenant_id = generate_test_tenant();
    let write_off_id = Uuid::new_v4();

    let (_customer_id, invoice_id) = create_test_invoice(&pool, &tenant_id, 15000)
        .await
        .expect("failed to create test invoice");

    let req = WriteOffInvoiceRequest {
        write_off_id,
        app_id: tenant_id.clone(),
        invoice_id,
        customer_id: format!("cust-{}", tenant_id),
        written_off_amount_minor: 15000,
        currency: "usd".to_string(),
        reason: "uncollectable".to_string(),
        authorized_by: Some("finance-team".to_string()),
        correlation_id: Uuid::new_v4().to_string(),
        causation_id: None,
    };

    let result = write_off_invoice(&pool, req)
        .await
        .expect("write_off_invoice failed");

    match result {
        WriteOffInvoiceResult::WrittenOff {
            write_off_row_id,
            write_off_id: returned_id,
            ..
        } => {
            assert_eq!(returned_id, write_off_id);
            assert!(write_off_row_id > 0);

            // Verify outbox event was written atomically
            let event_count = count_outbox_events(&pool, &write_off_id.to_string())
                .await
                .expect("failed to count outbox events");
            assert_eq!(event_count, 1, "expected exactly 1 outbox event");

            // Verify event type
            let event_type: String =
                sqlx::query_scalar("SELECT event_type FROM events_outbox WHERE aggregate_id = $1")
                    .bind(write_off_id.to_string())
                    .fetch_one(&pool)
                    .await
                    .expect("failed to fetch event type");
            assert_eq!(event_type, "ar.invoice_written_off");
        }
        other => panic!("expected WrittenOff, got {:?}", other),
    }

    cleanup_tenant(&pool, &tenant_id).await.unwrap();
}

/// Test: duplicate write_off_id is a no-op (idempotency).
#[tokio::test]
async fn test_write_off_idempotency() {
    let pool = get_ar_pool().await;
    let tenant_id = generate_test_tenant();
    let write_off_id = Uuid::new_v4();

    let (_customer_id, invoice_id) = create_test_invoice(&pool, &tenant_id, 9900)
        .await
        .expect("failed to create test invoice");

    let make_req = || WriteOffInvoiceRequest {
        write_off_id,
        app_id: tenant_id.clone(),
        invoice_id,
        customer_id: format!("cust-{}", tenant_id),
        written_off_amount_minor: 9900,
        currency: "usd".to_string(),
        reason: "uncollectable".to_string(),
        authorized_by: None,
        correlation_id: Uuid::new_v4().to_string(),
        causation_id: None,
    };

    // First write-off: should succeed
    let first = write_off_invoice(&pool, make_req())
        .await
        .expect("first write-off failed");
    assert!(
        matches!(first, WriteOffInvoiceResult::WrittenOff { .. }),
        "expected WrittenOff on first call"
    );

    // Second write-off with same write_off_id: should be AlreadyProcessed
    let second = write_off_invoice(&pool, make_req())
        .await
        .expect("second write-off failed");
    assert!(
        matches!(second, WriteOffInvoiceResult::AlreadyProcessed { .. }),
        "expected AlreadyProcessed on second call"
    );

    // Only one write-off row should exist
    let row_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM ar_invoice_write_offs WHERE write_off_id = $1")
            .bind(write_off_id)
            .fetch_one(&pool)
            .await
            .expect("failed to count write-off rows");
    assert_eq!(row_count, 1, "expected exactly 1 write-off row");

    // Only one outbox event should exist
    let event_count = count_outbox_events(&pool, &write_off_id.to_string())
        .await
        .expect("failed to count outbox events");
    assert_eq!(
        event_count, 1,
        "expected exactly 1 outbox event (no duplicate)"
    );

    cleanup_tenant(&pool, &tenant_id).await.unwrap();
}

/// Test: second write-off on same invoice (different write_off_id) returns AlreadyWrittenOff.
#[tokio::test]
async fn test_double_write_off_same_invoice_returns_already_written_off() {
    let pool = get_ar_pool().await;
    let tenant_id = generate_test_tenant();

    let (_customer_id, invoice_id) = create_test_invoice(&pool, &tenant_id, 5000)
        .await
        .expect("failed to create test invoice");

    // First write-off: should succeed
    let first_req = WriteOffInvoiceRequest {
        write_off_id: Uuid::new_v4(),
        app_id: tenant_id.clone(),
        invoice_id,
        customer_id: format!("cust-{}", tenant_id),
        written_off_amount_minor: 5000,
        currency: "usd".to_string(),
        reason: "uncollectable".to_string(),
        authorized_by: None,
        correlation_id: Uuid::new_v4().to_string(),
        causation_id: None,
    };
    write_off_invoice(&pool, first_req)
        .await
        .expect("first write-off failed");

    // Second write-off on same invoice but different write_off_id: AlreadyWrittenOff
    let second_req = WriteOffInvoiceRequest {
        write_off_id: Uuid::new_v4(), // different ID — not an idempotency replay
        app_id: tenant_id.clone(),
        invoice_id,
        customer_id: format!("cust-{}", tenant_id),
        written_off_amount_minor: 5000,
        currency: "usd".to_string(),
        reason: "duplicate_attempt".to_string(),
        authorized_by: None,
        correlation_id: Uuid::new_v4().to_string(),
        causation_id: None,
    };
    let result = write_off_invoice(&pool, second_req)
        .await
        .expect("second write-off call failed unexpectedly");

    assert!(
        matches!(result, WriteOffInvoiceResult::AlreadyWrittenOff { .. }),
        "expected AlreadyWrittenOff for double write-off, got {:?}",
        result
    );

    // Only one write-off row should exist for this invoice
    let row_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM ar_invoice_write_offs WHERE invoice_id = $1")
            .bind(invoice_id)
            .fetch_one(&pool)
            .await
            .expect("count failed");
    assert_eq!(row_count, 1, "exactly one write-off per invoice");

    cleanup_tenant(&pool, &tenant_id).await.unwrap();
}

/// Test: zero or negative amount is rejected before any DB operation.
#[tokio::test]
async fn test_write_off_invalid_amount_rejected() {
    let pool = get_ar_pool().await;
    let tenant_id = generate_test_tenant();

    let (_customer_id, invoice_id) = create_test_invoice(&pool, &tenant_id, 10000)
        .await
        .expect("failed to create test invoice");

    let req = WriteOffInvoiceRequest {
        write_off_id: Uuid::new_v4(),
        app_id: tenant_id.clone(),
        invoice_id,
        customer_id: "cust-x".to_string(),
        written_off_amount_minor: 0,
        currency: "usd".to_string(),
        reason: "test".to_string(),
        authorized_by: None,
        correlation_id: Uuid::new_v4().to_string(),
        causation_id: None,
    };

    let err = write_off_invoice(&pool, req)
        .await
        .expect_err("expected InvalidAmount error");
    assert!(
        matches!(err, ar_rs::write_offs::WriteOffError::InvalidAmount(0)),
        "expected InvalidAmount(0), got {:?}",
        err
    );

    // No write-off rows should have been inserted
    let row_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM ar_invoice_write_offs WHERE app_id = $1")
            .bind(&tenant_id)
            .fetch_one(&pool)
            .await
            .expect("count failed");
    assert_eq!(row_count, 0, "no rows expected after rejection");

    cleanup_tenant(&pool, &tenant_id).await.unwrap();
}

/// Test: unknown invoice_id is rejected with InvoiceNotFound.
#[tokio::test]
async fn test_write_off_unknown_invoice_rejected() {
    let pool = get_ar_pool().await;
    let tenant_id = generate_test_tenant();

    let req = WriteOffInvoiceRequest {
        write_off_id: Uuid::new_v4(),
        app_id: tenant_id.clone(),
        invoice_id: i32::MAX, // very unlikely to exist
        customer_id: "cust-x".to_string(),
        written_off_amount_minor: 1000,
        currency: "usd".to_string(),
        reason: "test".to_string(),
        authorized_by: None,
        correlation_id: Uuid::new_v4().to_string(),
        causation_id: None,
    };

    let err = write_off_invoice(&pool, req)
        .await
        .expect_err("expected InvoiceNotFound error");
    assert!(
        matches!(
            err,
            ar_rs::write_offs::WriteOffError::InvoiceNotFound { .. }
        ),
        "expected InvoiceNotFound, got {:?}",
        err
    );

    cleanup_tenant(&pool, &tenant_id).await.unwrap();
}

/// Test: write-off row and outbox event are committed atomically
/// and outbox event carries mutation_class = REVERSAL.
#[tokio::test]
async fn test_write_off_outbox_atomicity_and_reversal_class() {
    let pool = get_ar_pool().await;
    let tenant_id = generate_test_tenant();
    let write_off_id = Uuid::new_v4();

    let (_customer_id, invoice_id) = create_test_invoice(&pool, &tenant_id, 20000)
        .await
        .expect("failed to create test invoice");

    let req = WriteOffInvoiceRequest {
        write_off_id,
        app_id: tenant_id.clone(),
        invoice_id,
        customer_id: format!("cust-{}", tenant_id),
        written_off_amount_minor: 20000,
        currency: "usd".to_string(),
        reason: "bankruptcy".to_string(),
        authorized_by: Some("legal-team".to_string()),
        correlation_id: Uuid::new_v4().to_string(),
        causation_id: Some("dunning-final-escalation".to_string()),
    };

    write_off_invoice(&pool, req)
        .await
        .expect("write_off_invoice failed");

    // Both the write-off row and the outbox event must be present
    let write_off_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM ar_invoice_write_offs WHERE write_off_id = $1")
            .bind(write_off_id)
            .fetch_one(&pool)
            .await
            .expect("count failed");

    let event_count = count_outbox_events(&pool, &write_off_id.to_string())
        .await
        .expect("count failed");

    assert_eq!(
        write_off_count, 1,
        "write-off row must exist after write-off"
    );
    assert_eq!(
        event_count, 1,
        "outbox event must exist atomically with write-off row"
    );

    // Verify outbox event has mutation_class = REVERSAL (not DATA_MUTATION)
    let mutation_class: String =
        sqlx::query_scalar("SELECT mutation_class FROM events_outbox WHERE aggregate_id = $1")
            .bind(write_off_id.to_string())
            .fetch_one(&pool)
            .await
            .expect("fetch mutation_class failed");
    assert_eq!(
        mutation_class, "REVERSAL",
        "write-off must be a REVERSAL, not DATA_MUTATION"
    );

    // Verify outbox event has correlation/causation propagated
    let (correlation_id, causation_id): (String, Option<String>) = sqlx::query_as(
        "SELECT correlation_id, causation_id FROM events_outbox WHERE aggregate_id = $1",
    )
    .bind(write_off_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("fetch correlation/causation failed");
    assert!(!correlation_id.is_empty(), "correlation_id must be set");
    assert_eq!(
        causation_id.as_deref(),
        Some("dunning-final-escalation"),
        "causation_id must be propagated"
    );

    cleanup_tenant(&pool, &tenant_id).await.unwrap();
}
