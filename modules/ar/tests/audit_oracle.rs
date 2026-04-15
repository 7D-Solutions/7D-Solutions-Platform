//! Audit Oracle — AR module
//!
//! Asserts that every AR mutation (create_invoice, finalize_invoice) writes
//! exactly one audit_events row inside the same transaction as the mutation.
//!
//! Real database, no mocks. Run:
//!   ./scripts/cargo-slot.sh test -p ar-rs audit_oracle -- --nocapture

mod common;

use ar_rs::domain::invoices::service::{create_invoice, finalize_invoice};
use ar_rs::models::{CreateInvoiceRequest, FinalizeInvoiceRequest};
use event_bus::TracingContext;
use serial_test::serial;

const APP_ID: &str = "00000000-0000-0000-0000-000000000001";

/// Count audit_events rows for a given entity_id + action.
async fn count_audit_events(pool: &sqlx::PgPool, entity_id: &str, action: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM audit_events WHERE entity_id = $1 AND action = $2",
    )
    .bind(entity_id)
    .bind(action)
    .fetch_one(pool)
    .await
    .expect("count audit_events")
}

/// Fetch mutation_class for a given entity_id + action.
async fn fetch_mutation_class(pool: &sqlx::PgPool, entity_id: &str, action: &str) -> String {
    sqlx::query_scalar::<_, String>(
        "SELECT mutation_class::text FROM audit_events WHERE entity_id = $1 AND action = $2 LIMIT 1",
    )
    .bind(entity_id)
    .bind(action)
    .fetch_one(pool)
    .await
    .expect("fetch mutation_class")
}

// ============================================================================
// 1. create_invoice → exactly 1 CREATE audit record
// ============================================================================

#[tokio::test]
#[serial]
async fn audit_oracle_create_invoice() {
    let pool = common::setup_pool().await;
    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;

    let req = CreateInvoiceRequest {
        ar_customer_id: customer_id,
        subscription_id: None,
        status: None,
        amount_cents: 5000,
        currency: Some("usd".to_string()),
        due_at: None,
        metadata: None,
        billing_period_start: None,
        billing_period_end: None,
        line_item_details: None,
        compliance_codes: None,
        correlation_id: None,
        party_id: None,
    };

    let invoice = create_invoice(&pool, APP_ID, None, TracingContext::new(), req, None)
        .await
        .expect("create_invoice");

    let entity_id = invoice.id.to_string();

    let count = count_audit_events(&pool, &entity_id, "CreateInvoice").await;
    assert_eq!(
        count, 1,
        "Expected exactly 1 audit record for CreateInvoice"
    );

    let mc = fetch_mutation_class(&pool, &entity_id, "CreateInvoice").await;
    assert_eq!(mc, "CREATE", "mutation_class should be CREATE");

    let actor_id: Option<String> = sqlx::query_scalar(
        "SELECT actor_id::text FROM audit_events WHERE entity_id = $1 AND action = $2 LIMIT 1",
    )
    .bind(&entity_id)
    .bind("CreateInvoice")
    .fetch_one(&pool)
    .await
    .expect("fetch actor_id");
    assert_eq!(
        actor_id.unwrap_or_default(),
        "00000000-0000-0000-0000-000000000000",
        "actor_id should be nil UUID for system writes"
    );

    common::teardown_pool(pool).await;
}

// ============================================================================
// 2. finalize_invoice → exactly 1 STATE_TRANSITION audit record
// ============================================================================

#[tokio::test]
#[serial]
async fn audit_oracle_finalize_invoice() {
    let pool = common::setup_pool().await;
    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;

    // Create a draft invoice first
    let req = CreateInvoiceRequest {
        ar_customer_id: customer_id,
        subscription_id: None,
        status: Some("draft".to_string()),
        amount_cents: 7500,
        currency: Some("usd".to_string()),
        due_at: None,
        metadata: None,
        billing_period_start: None,
        billing_period_end: None,
        line_item_details: None,
        compliance_codes: None,
        correlation_id: None,
        party_id: None,
    };

    let invoice = create_invoice(&pool, APP_ID, None, TracingContext::new(), req, None)
        .await
        .expect("create_invoice");

    let invoice_id = invoice.id;

    // Finalize the invoice
    finalize_invoice(
        &pool,
        APP_ID,
        TracingContext::new(),
        invoice_id,
        FinalizeInvoiceRequest { paid_at: None },
    )
    .await
    .expect("finalize_invoice");

    let entity_id = invoice_id.to_string();

    let count = count_audit_events(&pool, &entity_id, "FinalizeInvoice").await;
    assert_eq!(
        count, 1,
        "Expected exactly 1 audit record for FinalizeInvoice"
    );

    let mc = fetch_mutation_class(&pool, &entity_id, "FinalizeInvoice").await;
    assert_eq!(
        mc, "STATE_TRANSITION",
        "mutation_class should be STATE_TRANSITION"
    );

    common::teardown_pool(pool).await;
}
