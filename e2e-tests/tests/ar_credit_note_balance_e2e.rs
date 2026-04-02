//! E2E Test: AR Credit Note — Balance Reduction, NATS Event, Over-credit Guard (bd-35dm)
//!
//! **Coverage:**
//! 1. Partial credit reduces computed invoice balance (amount_cents - SUM credits = expected balance)
//! 2. Credit note issued event published to NATS with correct amount_minor and tenant_id
//! 3. Over-credit guard: credit > invoice amount returns OverCreditBalance (guard enforces no over-crediting)
//!
//! **Invariant:** Total credits against an invoice must never exceed invoice amount_cents.
//!
//! **Pattern:** No Docker, no mocks — direct library calls + real DB + real NATS (localhost:4222)

mod common;

use anyhow::Result;
use ar_rs::credit_notes::{issue_credit_note, CreditNoteError, IssueCreditNoteRequest};
use common::{generate_test_tenant, get_ar_pool};
use futures::StreamExt;
use std::time::Duration;
use uuid::Uuid;

// ============================================================================
// Helpers
// ============================================================================

async fn create_customer_and_invoice(
    pool: &sqlx::PgPool,
    tenant_id: &str,
    amount_cents: i64,
) -> Result<(i32, i32)> {
    let customer_id: i32 = sqlx::query_scalar(
        "INSERT INTO ar_customers (app_id, email, name, status, retry_attempt_count, created_at, updated_at)
         VALUES ($1, $2, $3, 'active', 0, NOW(), NOW()) RETURNING id",
    )
    .bind(tenant_id)
    .bind(format!("bal-test-{}@test.local", Uuid::new_v4()))
    .bind("Balance Test Customer")
    .fetch_one(pool)
    .await?;

    let invoice_id: i32 = sqlx::query_scalar(
        "INSERT INTO ar_invoices (app_id, tilled_invoice_id, ar_customer_id, status, amount_cents, currency, created_at, updated_at)
         VALUES ($1, $2, $3, 'open', $4, 'usd', NOW(), NOW()) RETURNING id",
    )
    .bind(tenant_id)
    .bind(format!("in_bal_{}", Uuid::new_v4()))
    .bind(customer_id)
    .bind(amount_cents)
    .fetch_one(pool)
    .await?;

    Ok((customer_id, invoice_id))
}

async fn cleanup(pool: &sqlx::PgPool, tenant_id: &str) -> Result<()> {
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

fn make_req(tenant_id: &str, invoice_id: i32, amount_minor: i64) -> IssueCreditNoteRequest {
    IssueCreditNoteRequest {
        credit_note_id: Uuid::new_v4(),
        app_id: tenant_id.to_string(),
        customer_id: format!("cust-{}", tenant_id),
        invoice_id,
        amount_minor,
        currency: "usd".to_string(),
        reason: "partial_credit".to_string(),
        reference_id: None,
        issued_by: None,
        correlation_id: Uuid::new_v4().to_string(),
        causation_id: None,
    }
}

// ============================================================================
// Tests
// ============================================================================

/// Test: issuing a partial credit reduces the computed invoice balance.
/// Invoice amount_cents=100000, credit amount_minor=25000 → balance=75000.
#[tokio::test]
async fn test_credit_note_reduces_invoice_balance() {
    let pool = get_ar_pool().await;
    let tenant_id = generate_test_tenant();

    let (_customer_id, invoice_id) = create_customer_and_invoice(&pool, &tenant_id, 100000)
        .await
        .expect("create invoice failed");

    issue_credit_note(&pool, make_req(&tenant_id, invoice_id, 25000))
        .await
        .expect("issue_credit_note failed");

    // Balance = invoice.amount_cents - SUM(credit_notes.amount_minor for this invoice)
    let balance: i64 = sqlx::query_scalar(
        "SELECT i.amount_cents::BIGINT - COALESCE(SUM(cn.amount_minor), 0)::BIGINT
         FROM ar_invoices i
         LEFT JOIN ar_credit_notes cn ON cn.invoice_id = i.id AND cn.app_id = i.app_id
         WHERE i.id = $1 AND i.app_id = $2
         GROUP BY i.amount_cents",
    )
    .bind(invoice_id)
    .bind(&tenant_id)
    .fetch_one(&pool)
    .await
    .expect("balance query failed");

    assert_eq!(balance, 75000, "balance = 100000 - 25000 = 75000");

    cleanup(&pool, &tenant_id).await.unwrap();
}

/// Test: credit note issued event published to NATS with correct amount_minor and tenant_id.
/// Subscribe before issuing, manually flush AR outbox to NATS, verify envelope fields.
#[tokio::test]
async fn test_credit_note_nats_event_payload() {
    let pool = get_ar_pool().await;
    let tenant_id = generate_test_tenant();

    let (_customer_id, invoice_id) = create_customer_and_invoice(&pool, &tenant_id, 50000)
        .await
        .expect("create invoice failed");

    // Subscribe before issuing so we don't miss the message
    let nats = common::setup_nats_client().await;
    let nats_subject = "ar.events.ar.credit_note_issued";
    let mut sub = common::subscribe_to_events(&nats, nats_subject).await;

    issue_credit_note(&pool, make_req(&tenant_id, invoice_id, 15000))
        .await
        .expect("issue_credit_note failed");

    // Manually publish the outbox events for this tenant to NATS
    let events = ar_rs::events::outbox::fetch_unpublished_events(&pool, 100)
        .await
        .expect("fetch_unpublished_events failed");
    for event in events {
        if event.event_type == "ar.credit_note_issued"
            && event.tenant_id.as_deref() == Some(&tenant_id)
        {
            let subject = format!("ar.events.{}", event.event_type);
            let payload_bytes = serde_json::to_vec(&event.payload).expect("serialize failed");
            nats.publish(subject, payload_bytes.into())
                .await
                .expect("nats publish failed");
            ar_rs::events::outbox::mark_as_published(&pool, event.event_id)
                .await
                .expect("mark_as_published failed");
        }
    }

    // Receive and verify
    let msg = tokio::time::timeout(Duration::from_secs(5), sub.next())
        .await
        .expect("timeout: no NATS message within 5s")
        .expect("NATS subscriber closed");

    let envelope: serde_json::Value =
        serde_json::from_slice(&msg.payload).expect("deserialize envelope failed");

    assert_eq!(
        envelope["tenant_id"].as_str().unwrap_or(""),
        tenant_id,
        "envelope.tenant_id must match"
    );

    let payload = &envelope["payload"];
    assert_eq!(
        payload["amount_minor"].as_i64().unwrap_or(0),
        15000,
        "payload.amount_minor must be 15000"
    );
    assert_eq!(
        payload["tenant_id"].as_str().unwrap_or(""),
        tenant_id,
        "payload.tenant_id must match"
    );

    cleanup(&pool, &tenant_id).await.unwrap();
}

/// Test: credit > invoice amount is rejected — over-credit guard enforced.
/// Invoice amount_cents=10000, credit attempt=15000 → OverCreditBalance error.
/// No credit note row is inserted.
#[tokio::test]
async fn test_over_credit_rejected() {
    let pool = get_ar_pool().await;
    let tenant_id = generate_test_tenant();

    let (_customer_id, invoice_id) = create_customer_and_invoice(&pool, &tenant_id, 10000)
        .await
        .expect("create invoice failed");

    let err = issue_credit_note(&pool, make_req(&tenant_id, invoice_id, 15000))
        .await
        .expect_err("expected OverCreditBalance error");

    assert!(
        matches!(
            err,
            CreditNoteError::OverCreditBalance {
                invoice_amount_cents: 10000,
                requested: 15000,
                ..
            }
        ),
        "expected OverCreditBalance(invoice=10000, requested=15000), got {:?}",
        err
    );

    // Guard: no row inserted on rejection
    let row_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM ar_credit_notes WHERE app_id = $1")
            .bind(&tenant_id)
            .fetch_one(&pool)
            .await
            .expect("count failed");
    assert_eq!(
        row_count, 0,
        "no credit note row must exist after over-credit rejection"
    );

    cleanup(&pool, &tenant_id).await.unwrap();
}
