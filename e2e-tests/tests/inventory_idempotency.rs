//! E2E Test: Inventory Idempotency — No Double-Move or Double-Post (bd-2x3v)
//!
//! ## Coverage
//! 1. Duplicate receipt (same idempotency_key) → second call returns stored result (replay).
//!    No duplicate ledger row, no duplicate layer, no duplicate outbox event.
//! 2. Duplicate reserve (same idempotency_key) → replay. quantity_reserved not doubled.
//! 3. Duplicate issue (same idempotency_key) → replay. No duplicate ledger row.
//! 4. Full pipeline idempotency: receipt → issue → GL; then replay receipt + replay issue.
//!    GL posting uses original event_id → DuplicateEvent. Stock not double-consumed.
//! 5. Conflicting idempotency key (different payload) → ConflictingIdempotencyKey error.
//!
//! ## Pattern
//! No Docker, no mocks — live inventory DB and live GL DB.

mod common;

use anyhow::Result;
use common::{generate_test_tenant, get_gl_pool};
use gl_rs::consumer::gl_inventory_consumer::{
    process_inventory_cogs_posting,
    ConsumedLayer as GlConsumedLayer, ItemIssuedPayload as GlItemIssuedPayload,
    SourceRef as GlSourceRef,
};
use inventory_rs::domain::{
    issue_service::{IssueError, IssueRequest, process_issue},
    items::{CreateItemRequest, ItemRepo, TrackingMode},
    receipt_service::{ReceiptRequest, process_receipt},
    reservation_service::{ReserveRequest, process_reserve},
};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

// ============================================================================
// Pool helpers
// ============================================================================

async fn get_inventory_pool() -> sqlx::PgPool {
    let url = std::env::var("INVENTORY_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| {
            "postgresql://inventory_user:inventory_pass@localhost:5442/inventory_db".to_string()
        });

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to inventory DB");

    sqlx::migrate!("../modules/inventory/db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run inventory migrations");

    pool
}

// ============================================================================
// GL helpers (from inventory_gl_e2e.rs pattern)
// ============================================================================

async fn setup_gl_accounts(pool: &sqlx::PgPool, tenant_id: &str) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO accounts (id, tenant_id, code, name, type, normal_balance, is_active)
        VALUES
            (gen_random_uuid(), $1, 'COGS', 'Cost of Goods Sold', 'expense', 'debit', true),
            (gen_random_uuid(), $1, 'INVENTORY', 'Inventory Asset', 'asset', 'debit', true)
        ON CONFLICT (tenant_id, code) DO NOTHING
        "#,
    )
    .bind(tenant_id)
    .execute(pool)
    .await?;
    Ok(())
}

async fn setup_open_period(pool: &sqlx::PgPool, tenant_id: &str) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO accounting_periods (tenant_id, period_start, period_end, is_closed)
        VALUES ($1, '2026-01-01', '2026-12-31', false)
        ON CONFLICT DO NOTHING
        "#,
    )
    .bind(tenant_id)
    .execute(pool)
    .await?;
    Ok(())
}

// ============================================================================
// Cleanup
// ============================================================================

async fn cleanup_inventory(pool: &sqlx::PgPool, tenant_id: &str) {
    for q in [
        "DELETE FROM inv_outbox WHERE tenant_id = $1",
        "DELETE FROM inv_idempotency_keys WHERE tenant_id = $1",
        "DELETE FROM layer_consumptions WHERE ledger_entry_id IN (SELECT id FROM inventory_ledger WHERE tenant_id = $1)",
        "DELETE FROM inventory_serial_instances WHERE tenant_id = $1",
        "DELETE FROM item_on_hand WHERE tenant_id = $1",
        "DELETE FROM inventory_reservations WHERE tenant_id = $1",
        "DELETE FROM inv_adjustments WHERE tenant_id = $1",
        "DELETE FROM inventory_layers WHERE tenant_id = $1",
        "DELETE FROM inventory_ledger WHERE tenant_id = $1",
        "DELETE FROM inventory_lots WHERE tenant_id = $1",
        "DELETE FROM items WHERE tenant_id = $1",
    ] {
        sqlx::query(q).bind(tenant_id).execute(pool).await.ok();
    }
}

async fn cleanup_gl(pool: &sqlx::PgPool, tenant_id: &str) {
    for q in [
        "DELETE FROM journal_lines WHERE journal_entry_id IN (SELECT id FROM journal_entries WHERE tenant_id = $1)",
        "DELETE FROM processed_events WHERE event_id IN (SELECT source_event_id FROM journal_entries WHERE tenant_id = $1)",
        "DELETE FROM journal_entries WHERE tenant_id = $1",
        "DELETE FROM account_balances WHERE tenant_id = $1",
        "DELETE FROM period_summary_snapshots WHERE tenant_id = $1",
        "DELETE FROM accounts WHERE tenant_id = $1",
        "DELETE FROM accounting_periods WHERE tenant_id = $1",
    ] {
        sqlx::query(q).bind(tenant_id).execute(pool).await.ok();
    }
}

// ============================================================================
// Helper: build GL payload from issue result
// ============================================================================

fn to_gl_payload(
    issue: &inventory_rs::domain::issue_service::IssueResult,
    sku: &str,
) -> GlItemIssuedPayload {
    GlItemIssuedPayload {
        issue_line_id: issue.issue_line_id,
        tenant_id: issue.tenant_id.clone(),
        item_id: issue.item_id,
        sku: sku.to_string(),
        warehouse_id: issue.warehouse_id,
        quantity: issue.quantity,
        total_cost_minor: issue.total_cost_minor,
        currency: issue.currency.clone(),
        consumed_layers: issue
            .consumed_layers
            .iter()
            .map(|cl| GlConsumedLayer {
                layer_id: cl.layer_id,
                quantity: cl.quantity,
                unit_cost_minor: cl.unit_cost_minor,
                extended_cost_minor: cl.extended_cost_minor,
            })
            .collect(),
        source_ref: GlSourceRef {
            source_module: issue.source_ref.source_module.clone(),
            source_type: issue.source_ref.source_type.clone(),
            source_id: issue.source_ref.source_id.clone(),
            source_line_id: issue.source_ref.source_line_id.clone(),
        },
        issued_at: issue.issued_at,
    }
}

// ============================================================================
// Test 1: Duplicate receipt — replay returns stored result, no double row
// ============================================================================

#[tokio::test]
#[serial]
async fn inventory_idempotency_duplicate_receipt_is_replay() {
    let pool = get_inventory_pool().await;
    let tenant_id = format!("e2e-{}", Uuid::new_v4());

    let item = ItemRepo::create(
        &pool,
        &CreateItemRequest {
            tenant_id: tenant_id.clone(),
            sku: "E2E-IDEM-RCPT-001".to_string(),
            name: "Idempotency Receipt Item".to_string(),
            description: None,
            inventory_account_ref: "1200".to_string(),
            cogs_account_ref: "5000".to_string(),
            variance_account_ref: "5010".to_string(),
            uom: None,
            tracking_mode: TrackingMode::None,
        },
    )
    .await
    .expect("create item");

    let idempotency_key = format!("rcpt-idem-{}", Uuid::new_v4());
    let req = ReceiptRequest {
        tenant_id: tenant_id.clone(),
        item_id: item.id,
        warehouse_id: Uuid::new_v4(),
        location_id: None,
        quantity: 100,
        unit_cost_minor: 1000,
        currency: "usd".to_string(),
        purchase_order_id: None,
        idempotency_key: idempotency_key.clone(),
        correlation_id: None,
        causation_id: None,
        lot_code: None,
        serial_codes: None,
        uom_id: None,
    };

    let (r1, is_replay1) = process_receipt(&pool, &req).await.expect("first receipt");
    assert!(!is_replay1, "first call must not be replay");

    let (r2, is_replay2) = process_receipt(&pool, &req).await.expect("second receipt");
    assert!(is_replay2, "second call must be replay");

    // Stored result must be identical
    assert_eq!(r1.receipt_line_id, r2.receipt_line_id, "same receipt_line_id");
    assert_eq!(r1.layer_id, r2.layer_id, "same layer_id");
    assert_eq!(r1.event_id, r2.event_id, "same event_id");
    assert_eq!(r1.quantity, r2.quantity, "same quantity");

    // No duplicate rows
    let ledger_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inventory_ledger WHERE tenant_id = $1",
    )
    .bind(&tenant_id)
    .fetch_one(&pool)
    .await
    .expect("count");
    assert_eq!(ledger_count, 1, "no duplicate ledger rows after replay");

    let layer_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inventory_layers WHERE tenant_id = $1",
    )
    .bind(&tenant_id)
    .fetch_one(&pool)
    .await
    .expect("count");
    assert_eq!(layer_count, 1, "no duplicate FIFO layers after replay");

    let outbox_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inv_outbox WHERE tenant_id = $1",
    )
    .bind(&tenant_id)
    .fetch_one(&pool)
    .await
    .expect("count");
    assert_eq!(outbox_count, 1, "no duplicate outbox events after replay");

    cleanup_inventory(&pool, &tenant_id).await;
}

// ============================================================================
// Test 2: Duplicate reserve — replay, quantity_reserved not doubled
// ============================================================================

#[tokio::test]
#[serial]
async fn inventory_idempotency_duplicate_reserve_is_replay() {
    let pool = get_inventory_pool().await;
    let tenant_id = format!("e2e-{}", Uuid::new_v4());
    let warehouse_id = Uuid::new_v4();

    let item = ItemRepo::create(
        &pool,
        &CreateItemRequest {
            tenant_id: tenant_id.clone(),
            sku: "E2E-IDEM-RSV-001".to_string(),
            name: "Idempotency Reserve Item".to_string(),
            description: None,
            inventory_account_ref: "1200".to_string(),
            cogs_account_ref: "5000".to_string(),
            variance_account_ref: "5010".to_string(),
            uom: None,
            tracking_mode: TrackingMode::None,
        },
    )
    .await
    .expect("create item");

    let req = ReserveRequest {
        tenant_id: tenant_id.clone(),
        item_id: item.id,
        warehouse_id,
        quantity: 50,
        reference_type: Some("sales_order".to_string()),
        reference_id: Some("SO-IDEM-001".to_string()),
        expires_at: None,
        idempotency_key: format!("rsv-idem-{}", Uuid::new_v4()),
        correlation_id: None,
        causation_id: None,
    };

    let (r1, is_replay1) = process_reserve(&pool, &req).await.expect("first reserve");
    assert!(!is_replay1);

    let (r2, is_replay2) = process_reserve(&pool, &req).await.expect("second reserve");
    assert!(is_replay2, "second call must be replay");

    // Same result
    assert_eq!(r1.reservation_id, r2.reservation_id);
    assert_eq!(r1.event_id, r2.event_id);

    // quantity_reserved must not be doubled
    let qty_reserved: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(quantity_reserved), 0) FROM item_on_hand WHERE tenant_id = $1 AND item_id = $2",
    )
    .bind(&tenant_id)
    .bind(item.id)
    .fetch_one(&pool)
    .await
    .expect("quantity_reserved");
    assert_eq!(qty_reserved, 50, "quantity_reserved must not be doubled (expected 50, got {})", qty_reserved);

    // Only one reservation row
    let rsv_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inventory_reservations WHERE tenant_id = $1",
    )
    .bind(&tenant_id)
    .fetch_one(&pool)
    .await
    .expect("count");
    assert_eq!(rsv_count, 1, "no duplicate reservation rows");

    cleanup_inventory(&pool, &tenant_id).await;
}

// ============================================================================
// Test 3: Duplicate issue — replay, no double ledger or layer_consumption
// ============================================================================

#[tokio::test]
#[serial]
async fn inventory_idempotency_duplicate_issue_is_replay() {
    let pool = get_inventory_pool().await;
    let tenant_id = format!("e2e-{}", Uuid::new_v4());
    let warehouse_id = Uuid::new_v4();

    let item = ItemRepo::create(
        &pool,
        &CreateItemRequest {
            tenant_id: tenant_id.clone(),
            sku: "E2E-IDEM-ISSUE-001".to_string(),
            name: "Idempotency Issue Item".to_string(),
            description: None,
            inventory_account_ref: "1200".to_string(),
            cogs_account_ref: "5000".to_string(),
            variance_account_ref: "5010".to_string(),
            uom: None,
            tracking_mode: TrackingMode::None,
        },
    )
    .await
    .expect("create item");

    // Seed stock
    process_receipt(
        &pool,
        &ReceiptRequest {
            tenant_id: tenant_id.clone(),
            item_id: item.id,
            warehouse_id,
            location_id: None,
            quantity: 100,
            unit_cost_minor: 2000,
            currency: "usd".to_string(),
            purchase_order_id: None,
            idempotency_key: format!("rcpt-idem-issue-{}", Uuid::new_v4()),
            correlation_id: None,
            causation_id: None,
            lot_code: None,
            serial_codes: None,
            uom_id: None,
        },
    )
    .await
    .expect("seed stock");

    let issue_req = IssueRequest {
        tenant_id: tenant_id.clone(),
        item_id: item.id,
        warehouse_id,
        location_id: None,
        quantity: 25,
        currency: "usd".to_string(),
        source_module: "orders".to_string(),
        source_type: "sales_order".to_string(),
        source_id: "SO-IDEM-ISSUE-001".to_string(),
        source_line_id: None,
        idempotency_key: format!("issue-idem-{}", Uuid::new_v4()),
        correlation_id: None,
        causation_id: None,
        uom_id: None,
        lot_code: None,
        serial_codes: None,
    };

    let (i1, is_replay1) = process_issue(&pool, &issue_req).await.expect("first issue");
    assert!(!is_replay1);
    assert_eq!(i1.quantity, 25);

    let (i2, is_replay2) = process_issue(&pool, &issue_req).await.expect("second issue (replay)");
    assert!(is_replay2, "second issue must be replay");

    // Same result returned
    assert_eq!(i1.issue_line_id, i2.issue_line_id);
    assert_eq!(i1.event_id, i2.event_id);
    assert_eq!(i1.total_cost_minor, i2.total_cost_minor);

    // No duplicate ledger rows (1 for receipt + 1 for issue = 2 total)
    let ledger_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inventory_ledger WHERE tenant_id = $1",
    )
    .bind(&tenant_id)
    .fetch_one(&pool)
    .await
    .expect("count");
    assert_eq!(ledger_count, 2, "exactly 2 ledger rows (1 receipt + 1 issue)");

    // No duplicate layer_consumptions
    let consumption_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM layer_consumptions WHERE ledger_entry_id = $1",
    )
    .bind(i1.ledger_entry_id)
    .fetch_one(&pool)
    .await
    .expect("count");
    assert_eq!(consumption_count, 1, "exactly 1 layer_consumption row");

    // Layer remaining = 100 - 25 = 75 (not 100 - 50 = 50 if double-consumed)
    let remaining: i64 = sqlx::query_scalar(
        "SELECT SUM(quantity_remaining) FROM inventory_layers WHERE tenant_id = $1",
    )
    .bind(&tenant_id)
    .fetch_one(&pool)
    .await
    .expect("remaining");
    assert_eq!(remaining, 75, "remaining must be 75, not 50 (no double-consume)");

    cleanup_inventory(&pool, &tenant_id).await;
}

// ============================================================================
// Test 4: Full pipeline idempotency — replay at each step, no double GL post
// ============================================================================

/// Proves end-to-end idempotency:
/// 1. Receipt → issue → GL post (first time, all succeed)
/// 2. Replay receipt → same result, no new rows
/// 3. Replay issue → same result, no new rows
/// 4. Replay GL post (same event_id) → DuplicateEvent, no new journal entry
#[tokio::test]
#[serial]
async fn inventory_idempotency_full_pipeline_no_double_post() -> Result<()> {
    let inv_pool = get_inventory_pool().await;
    let gl_pool = get_gl_pool().await;
    let tenant_id = generate_test_tenant();
    let sku = "SKU-FULL-IDEM-001";

    setup_gl_accounts(&gl_pool, &tenant_id).await?;
    setup_open_period(&gl_pool, &tenant_id).await?;

    let item = ItemRepo::create(
        &inv_pool,
        &CreateItemRequest {
            tenant_id: tenant_id.clone(),
            sku: sku.to_string(),
            name: "Full Pipeline Idempotency Item".to_string(),
            description: None,
            inventory_account_ref: "INVENTORY".to_string(),
            cogs_account_ref: "COGS".to_string(),
            variance_account_ref: "COGS".to_string(),
            uom: None,
            tracking_mode: TrackingMode::None,
        },
    )
    .await
    .expect("create item");

    let warehouse_id = Uuid::new_v4();

    // --- Step 1: Receipt (first time) ---
    let rcpt_req = ReceiptRequest {
        tenant_id: tenant_id.clone(),
        item_id: item.id,
        warehouse_id,
        location_id: None,
        quantity: 100,
        unit_cost_minor: 3000, // $30.00
        currency: "usd".to_string(),
        purchase_order_id: None,
        idempotency_key: format!("rcpt-full-{}", Uuid::new_v4()),
        correlation_id: None,
        causation_id: None,
        lot_code: None,
        serial_codes: None,
        uom_id: None,
    };
    let (rcpt1, _) = process_receipt(&inv_pool, &rcpt_req).await.expect("receipt");

    // --- Step 2: Issue (first time) ---
    let issue_req = IssueRequest {
        tenant_id: tenant_id.clone(),
        item_id: item.id,
        warehouse_id,
        location_id: None,
        quantity: 10,
        currency: "usd".to_string(),
        source_module: "orders".to_string(),
        source_type: "sales_order".to_string(),
        source_id: "SO-FULL-IDEM-001".to_string(),
        source_line_id: None,
        idempotency_key: format!("issue-full-{}", Uuid::new_v4()),
        correlation_id: None,
        causation_id: None,
        uom_id: None,
        lot_code: None,
        serial_codes: None,
    };
    let (issue1, _) = process_issue(&inv_pool, &issue_req).await.expect("issue");
    assert_eq!(issue1.total_cost_minor, 300_000, "10 × $30 = $300");

    // --- Step 3: GL COGS post (first time) ---
    let gl_payload = to_gl_payload(&issue1, sku);
    let gl_event_id = issue1.event_id;

    process_inventory_cogs_posting(&gl_pool, gl_event_id, &tenant_id, "inventory", &gl_payload)
        .await
        .expect("GL post must succeed");

    let je_count1: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM journal_entries WHERE source_event_id = $1",
    )
    .bind(gl_event_id)
    .fetch_one(&gl_pool)
    .await?;
    assert_eq!(je_count1, 1, "exactly 1 journal entry after first GL post");

    // --- Step 4: Replay receipt ---
    let (rcpt2, is_replay) = process_receipt(&inv_pool, &rcpt_req).await.expect("replay receipt");
    assert!(is_replay, "second receipt must be replay");
    assert_eq!(rcpt1.receipt_line_id, rcpt2.receipt_line_id);
    assert_eq!(rcpt1.event_id, rcpt2.event_id);

    // No new ledger rows from replay (2 total: 1 receipt + 1 issue)
    let ledger_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inventory_ledger WHERE tenant_id = $1",
    )
    .bind(&tenant_id)
    .fetch_one(&inv_pool)
    .await?;
    assert_eq!(ledger_count, 2, "no new rows from replay receipt");

    // --- Step 5: Replay issue ---
    let (issue2, is_replay) = process_issue(&inv_pool, &issue_req).await.expect("replay issue");
    assert!(is_replay, "second issue must be replay");
    assert_eq!(issue1.issue_line_id, issue2.issue_line_id);
    assert_eq!(issue1.event_id, issue2.event_id);

    let ledger_count2: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inventory_ledger WHERE tenant_id = $1",
    )
    .bind(&tenant_id)
    .fetch_one(&inv_pool)
    .await?;
    assert_eq!(ledger_count2, 2, "no new rows from replay issue");

    // --- Step 6: Replay GL post with same event_id → DuplicateEvent ---
    let second_gl = process_inventory_cogs_posting(
        &gl_pool,
        gl_event_id,
        &tenant_id,
        "inventory",
        &gl_payload,
    )
    .await;

    assert!(
        matches!(second_gl, Err(gl_rs::services::journal_service::JournalError::DuplicateEvent(_))),
        "replay GL post must return DuplicateEvent, got: {:?}",
        second_gl,
    );

    // Still only one journal entry — no double post
    let je_count2: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM journal_entries WHERE source_event_id = $1",
    )
    .bind(gl_event_id)
    .fetch_one(&gl_pool)
    .await?;
    assert_eq!(je_count2, 1, "no duplicate journal entries after replay GL post");

    // Stock not double-consumed: remaining = 100 - 10 = 90 (not 80)
    let layer_remaining: i64 = sqlx::query_scalar(
        "SELECT SUM(quantity_remaining) FROM inventory_layers WHERE tenant_id = $1",
    )
    .bind(&tenant_id)
    .fetch_one(&inv_pool)
    .await?;
    assert_eq!(layer_remaining, 90, "layer remaining = 90 (no double-consume)");

    cleanup_inventory(&inv_pool, &tenant_id).await;
    cleanup_gl(&gl_pool, &tenant_id).await;
    Ok(())
}

// ============================================================================
// Test 5: Conflicting idempotency key (different payload) → error
// ============================================================================

/// A second receipt call with the same idempotency_key but different quantity
/// must return ConflictingIdempotencyKey, not silently overwrite.
#[tokio::test]
#[serial]
async fn inventory_idempotency_conflicting_key_rejected() {
    let pool = get_inventory_pool().await;
    let tenant_id = format!("e2e-{}", Uuid::new_v4());

    let item = ItemRepo::create(
        &pool,
        &CreateItemRequest {
            tenant_id: tenant_id.clone(),
            sku: "E2E-IDEM-CONFLICT-001".to_string(),
            name: "Conflicting Key Item".to_string(),
            description: None,
            inventory_account_ref: "1200".to_string(),
            cogs_account_ref: "5000".to_string(),
            variance_account_ref: "5010".to_string(),
            uom: None,
            tracking_mode: TrackingMode::None,
        },
    )
    .await
    .expect("create item");

    let idempotency_key = format!("rcpt-conflict-{}", Uuid::new_v4());
    let warehouse_id = Uuid::new_v4();

    // First call: quantity = 100
    let req1 = ReceiptRequest {
        tenant_id: tenant_id.clone(),
        item_id: item.id,
        warehouse_id,
        location_id: None,
        quantity: 100,
        unit_cost_minor: 1000,
        currency: "usd".to_string(),
        purchase_order_id: None,
        idempotency_key: idempotency_key.clone(),
        correlation_id: None,
        causation_id: None,
        lot_code: None,
        serial_codes: None,
        uom_id: None,
    };
    let (_, is_replay) = process_receipt(&pool, &req1).await.expect("first call");
    assert!(!is_replay);

    // Second call: different quantity (conflicting payload)
    let req2 = ReceiptRequest {
        quantity: 999, // different!
        ..req1
    };
    let err = process_receipt(&pool, &req2)
        .await
        .expect_err("conflicting key must fail");

    assert!(
        matches!(err, inventory_rs::domain::receipt_service::ReceiptError::ConflictingIdempotencyKey),
        "expected ConflictingIdempotencyKey, got: {:?}",
        err
    );

    // Stock must reflect only the first receipt (100, not 999)
    let on_hand: i64 = sqlx::query_scalar(
        "SELECT quantity_on_hand FROM item_on_hand WHERE tenant_id = $1 AND item_id = $2 AND warehouse_id = $3",
    )
    .bind(&tenant_id)
    .bind(item.id)
    .bind(warehouse_id)
    .fetch_one(&pool)
    .await
    .expect("on-hand");
    assert_eq!(on_hand, 100, "on-hand must remain 100 (conflicting receipt rejected)");

    cleanup_inventory(&pool, &tenant_id).await;
}

// ============================================================================
// Test 6: Duplicate issue after GL post — both idempotent, no extra GL entry
// ============================================================================

/// Proves that issuing twice (same idempotency_key) and posting GL twice
/// (same event_id) results in exactly 1 ledger row and 1 journal entry.
#[tokio::test]
#[serial]
async fn inventory_idempotency_issue_then_gl_no_double() -> Result<()> {
    let inv_pool = get_inventory_pool().await;
    let gl_pool = get_gl_pool().await;
    let tenant_id = generate_test_tenant();
    let sku = "SKU-DOUBLE-GUARD-001";

    setup_gl_accounts(&gl_pool, &tenant_id).await?;
    setup_open_period(&gl_pool, &tenant_id).await?;

    let item = ItemRepo::create(
        &inv_pool,
        &CreateItemRequest {
            tenant_id: tenant_id.clone(),
            sku: sku.to_string(),
            name: "Double Guard Item".to_string(),
            description: None,
            inventory_account_ref: "INVENTORY".to_string(),
            cogs_account_ref: "COGS".to_string(),
            variance_account_ref: "COGS".to_string(),
            uom: None,
            tracking_mode: TrackingMode::None,
        },
    )
    .await
    .expect("create item");

    let warehouse_id = Uuid::new_v4();

    process_receipt(
        &inv_pool,
        &ReceiptRequest {
            tenant_id: tenant_id.clone(),
            item_id: item.id,
            warehouse_id,
            location_id: None,
            quantity: 50,
            unit_cost_minor: 500,
            currency: "usd".to_string(),
            purchase_order_id: None,
            idempotency_key: format!("rcpt-dg-{}", Uuid::new_v4()),
            correlation_id: None,
            causation_id: None,
            lot_code: None,
            serial_codes: None,
            uom_id: None,
        },
    )
    .await
    .expect("seed stock");

    let issue_req = IssueRequest {
        tenant_id: tenant_id.clone(),
        item_id: item.id,
        warehouse_id,
        location_id: None,
        quantity: 20,
        currency: "usd".to_string(),
        source_module: "orders".to_string(),
        source_type: "sales_order".to_string(),
        source_id: "SO-DG-001".to_string(),
        source_line_id: None,
        idempotency_key: format!("issue-dg-{}", Uuid::new_v4()),
        correlation_id: None,
        causation_id: None,
        uom_id: None,
        lot_code: None,
        serial_codes: None,
    };

    // Issue twice (same idempotency_key)
    let (i1, _) = process_issue(&inv_pool, &issue_req).await.expect("issue 1");
    let (i2, is_replay) = process_issue(&inv_pool, &issue_req).await.expect("issue 2 (replay)");
    assert!(is_replay);
    assert_eq!(i1.event_id, i2.event_id);

    // GL post twice (same event_id)
    let gl_payload = to_gl_payload(&i1, sku);
    let event_id = i1.event_id;

    process_inventory_cogs_posting(&gl_pool, event_id, &tenant_id, "inventory", &gl_payload)
        .await
        .expect("GL post 1");

    let second = process_inventory_cogs_posting(
        &gl_pool,
        event_id,
        &tenant_id,
        "inventory",
        &gl_payload,
    )
    .await;
    assert!(
        matches!(second, Err(gl_rs::services::journal_service::JournalError::DuplicateEvent(_))),
        "GL post 2 must be DuplicateEvent"
    );

    // Exactly 1 journal entry
    let je_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM journal_entries WHERE source_event_id = $1",
    )
    .bind(event_id)
    .fetch_one(&gl_pool)
    .await?;
    assert_eq!(je_count, 1, "exactly 1 journal entry despite 2 GL posts");

    // Exactly 1 issue ledger row (receipt + issue = 2 total)
    let ledger_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inventory_ledger WHERE tenant_id = $1",
    )
    .bind(&tenant_id)
    .fetch_one(&inv_pool)
    .await?;
    assert_eq!(ledger_count, 2, "2 ledger rows: 1 receipt + 1 issue");

    // Layer remaining = 50 - 20 = 30 (not 50 - 40 = 10 if double-issued)
    let remaining: i64 = sqlx::query_scalar(
        "SELECT SUM(quantity_remaining) FROM inventory_layers WHERE tenant_id = $1",
    )
    .bind(&tenant_id)
    .fetch_one(&inv_pool)
    .await?;
    assert_eq!(remaining, 30, "layer remaining = 30 (no double-consume)");

    cleanup_inventory(&inv_pool, &tenant_id).await;
    cleanup_gl(&gl_pool, &tenant_id).await;
    Ok(())
}
