//! E2E Test: Inventory Receipt → Reserve → Issue → GL COGS Posting (bd-2x3v)
//!
//! ## Coverage
//! 1. Full pipeline: receipt creates on-hand stock, reserve holds quantity,
//!    issue consumes stock, GL posts balanced COGS entry — exactly-once.
//! 2. GL journal entry is balanced (DR COGS == CR INVENTORY).
//! 3. processed_events row created atomically with journal entry.
//! 4. Period-close protection: GL posting rejected when period is closed.
//! 5. Second GL post with same event_id returns DuplicateEvent (GL idempotency).
//!
//! ## Pattern
//! No Docker, no mocks — uses live inventory DB and live GL DB.
//! Inventory and GL each have their own connection pool.

mod common;

use anyhow::Result;
use chrono::Utc;
use common::{generate_test_tenant, get_gl_pool};
use gl_rs::consumers::gl_inventory_consumer::{
    process_inventory_cogs_posting, ConsumedLayer as GlConsumedLayer,
    ItemIssuedPayload as GlItemIssuedPayload, SourceRef as GlSourceRef,
};
use inventory_rs::domain::{
    issue_service::{process_issue, IssueRequest},
    items::{CreateItemRequest, ItemRepo, TrackingMode},
    receipt_service::{process_receipt, ReceiptRequest},
    reservation_service::{process_reserve, ReserveRequest},
};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

// ============================================================================
// Inventory pool helper (mirrors inventory_receipt_e2e.rs pattern)
// ============================================================================

async fn get_inventory_pool() -> sqlx::PgPool {
    let url = std::env::var("INVENTORY_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| {
            "postgresql://inventory_user:inventory_pass@localhost:5442/inventory_db".to_string()
        });

    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&url)
        .await
        .expect("Failed to connect to inventory DB — is INVENTORY_DATABASE_URL set?");

    sqlx::migrate!("../modules/inventory/db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run inventory migrations");

    pool
}

// ============================================================================
// GL helpers (mirrors inventory_gl_e2e.rs pattern)
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

async fn setup_closed_period(pool: &sqlx::PgPool, tenant_id: &str) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO accounting_periods
            (tenant_id, period_start, period_end, is_closed, closed_at, closed_by, close_hash)
        VALUES ($1, '2024-01-01', '2024-12-31', true, NOW(), 'test', 'test-close-hash-rrig')
        "#,
    )
    .bind(tenant_id)
    .execute(pool)
    .await?;
    Ok(())
}

async fn count_journal_entries(pool: &sqlx::PgPool, source_event_id: Uuid) -> Result<i64> {
    Ok(
        sqlx::query_scalar("SELECT COUNT(*) FROM journal_entries WHERE source_event_id = $1")
            .bind(source_event_id)
            .fetch_one(pool)
            .await?,
    )
}

async fn get_journal_lines(
    pool: &sqlx::PgPool,
    source_event_id: Uuid,
) -> Result<Vec<(String, i64, i64)>> {
    let entry_id: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM journal_entries WHERE source_event_id = $1 LIMIT 1")
            .bind(source_event_id)
            .fetch_optional(pool)
            .await?;

    let entry_id = match entry_id {
        Some(id) => id,
        None => return Ok(vec![]),
    };

    Ok(sqlx::query_as(
        r#"
        SELECT account_ref,
               COALESCE(debit_minor, 0) AS debit,
               COALESCE(credit_minor, 0) AS credit
        FROM journal_lines
        WHERE journal_entry_id = $1
        ORDER BY line_no
        "#,
    )
    .bind(entry_id)
    .fetch_all(pool)
    .await?)
}

// ============================================================================
// Cleanup helpers
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
        sqlx::query(q)
            .bind(tenant_id)
            .execute(pool)
            .await
            .ok();
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
        sqlx::query(q)
            .bind(tenant_id)
            .execute(pool)
            .await
            .ok();
    }
}

// ============================================================================
// Helpers: build GL payload from inventory issue result
// ============================================================================

fn to_gl_payload(
    issue_result: &inventory_rs::domain::issue_service::IssueResult,
    sku: &str,
) -> GlItemIssuedPayload {
    let consumed_layers: Vec<GlConsumedLayer> = issue_result
        .consumed_layers
        .iter()
        .map(|cl| GlConsumedLayer {
            layer_id: cl.layer_id,
            quantity: cl.quantity,
            unit_cost_minor: cl.unit_cost_minor,
            extended_cost_minor: cl.extended_cost_minor,
        })
        .collect();

    GlItemIssuedPayload {
        issue_line_id: issue_result.issue_line_id,
        tenant_id: issue_result.tenant_id.clone(),
        item_id: issue_result.item_id,
        sku: sku.to_string(),
        warehouse_id: issue_result.warehouse_id,
        quantity: issue_result.quantity,
        total_cost_minor: issue_result.total_cost_minor,
        currency: issue_result.currency.clone(),
        consumed_layers,
        source_ref: GlSourceRef {
            source_module: issue_result.source_ref.source_module.clone(),
            source_type: issue_result.source_ref.source_type.clone(),
            source_id: issue_result.source_ref.source_id.clone(),
            source_line_id: issue_result.source_ref.source_line_id.clone(),
        },
        issued_at: issue_result.issued_at,
    }
}

// ============================================================================
// Test 1: Full pipeline — receipt → reserve → issue → GL COGS (exactly-once)
// ============================================================================

/// Proves the complete inventory-to-GL pipeline:
/// - Receipt creates stock (100 units @ $25.00 = $2500.00 total)
/// - Reserve holds 50 units (leaving 50 available)
/// - Issue consumes 50 units → FIFO → total_cost_minor = 50 × 2500 = 125000
/// - GL posts DR COGS $1250.00 / CR INVENTORY $1250.00 atomically
/// - Second GL post with same event_id → DuplicateEvent (exactly-once)
#[tokio::test]
#[serial]
async fn inventory_full_pipeline_receipt_reserve_issue_gl_cogs() -> Result<()> {
    let inv_pool = get_inventory_pool().await;
    let gl_pool = get_gl_pool().await;
    let tenant_id = generate_test_tenant();
    let sku = "SKU-PIPELINE-001";

    // --- GL setup ---
    setup_gl_accounts(&gl_pool, &tenant_id).await?;
    setup_open_period(&gl_pool, &tenant_id).await?;

    // --- Inventory setup: create item ---
    let item = ItemRepo::create(
        &inv_pool,
        &CreateItemRequest {
            tenant_id: tenant_id.clone(),
            sku: sku.to_string(),
            name: "Pipeline Test Item".to_string(),
            description: None,
            inventory_account_ref: "INVENTORY".to_string(),
            cogs_account_ref: "COGS".to_string(),
            variance_account_ref: "COGS".to_string(),
            uom: None,
            tracking_mode: TrackingMode::None,
            make_buy: None,
        },
    )
    .await
    .expect("create item");

    let warehouse_id = Uuid::new_v4();

    // --- Step 1: Receipt — 100 units @ $25.00 ---
    let (receipt, is_replay) = process_receipt(
        &inv_pool,
        &ReceiptRequest {
            tenant_id: tenant_id.clone(),
            item_id: item.id,
            warehouse_id,
            location_id: None,
            quantity: 100,
            unit_cost_minor: 2500, // $25.00
            currency: "usd".to_string(),
            source_type: "purchase".to_string(),
            purchase_order_id: None,
            idempotency_key: format!("rcpt-pipeline-{}", Uuid::new_v4()),
            correlation_id: Some("e2e-pipeline".to_string()),
            causation_id: None,
            lot_code: None,
            serial_codes: None,
            uom_id: None,
        },
        None,
    )
    .await
    .expect("receipt must succeed");
    assert!(!is_replay, "must be a fresh receipt");
    assert_eq!(receipt.quantity, 100);

    // Verify on-hand after receipt (quantity_on_hand = 100)
    let on_hand: i64 = sqlx::query_scalar(
        "SELECT quantity_on_hand FROM item_on_hand WHERE tenant_id = $1 AND item_id = $2 AND warehouse_id = $3",
    )
    .bind(&tenant_id)
    .bind(item.id)
    .bind(warehouse_id)
    .fetch_one(&inv_pool)
    .await
    .expect("on-hand row after receipt");
    assert_eq!(on_hand, 100, "on-hand must be 100 after receipt");

    // --- Step 2: Reserve — 50 units ---
    let (reservation, is_replay) = process_reserve(
        &inv_pool,
        &ReserveRequest {
            tenant_id: tenant_id.clone(),
            item_id: item.id,
            warehouse_id,
            quantity: 50,
            reference_type: Some("sales_order".to_string()),
            reference_id: Some("SO-PIPELINE-001".to_string()),
            expires_at: None,
            idempotency_key: format!("rsv-pipeline-{}", Uuid::new_v4()),
            correlation_id: Some("e2e-pipeline".to_string()),
            causation_id: None,
        },
    )
    .await
    .expect("reserve must succeed");
    assert!(!is_replay);
    assert_eq!(reservation.quantity, 50);

    // Verify 50 reserved, 50 available
    let (qty_reserved, qty_available): (i64, i64) = sqlx::query_as(
        "SELECT quantity_reserved, quantity_available FROM item_on_hand WHERE tenant_id = $1 AND item_id = $2 AND warehouse_id = $3",
    )
    .bind(&tenant_id)
    .bind(item.id)
    .bind(warehouse_id)
    .fetch_one(&inv_pool)
    .await
    .expect("on-hand row after reserve");
    assert_eq!(qty_reserved, 50, "50 reserved");
    assert_eq!(qty_available, 50, "50 available (100 - 50)");

    // --- Step 3: Issue — 50 units from unreserved stock ---
    let (issue, is_replay) = process_issue(
        &inv_pool,
        &IssueRequest {
            tenant_id: tenant_id.clone(),
            item_id: item.id,
            warehouse_id,
            location_id: None,
            quantity: 50,
            currency: "usd".to_string(),
            source_module: "orders".to_string(),
            source_type: "sales_order".to_string(),
            source_id: "SO-PIPELINE-001".to_string(),
            source_line_id: Some("L1".to_string()),
            idempotency_key: format!("issue-pipeline-{}", Uuid::new_v4()),
            correlation_id: Some("e2e-pipeline".to_string()),
            causation_id: None,
            uom_id: None,
            lot_code: None,
            serial_codes: None,
        },
        None,
    )
    .await
    .expect("issue must succeed");
    assert!(!is_replay);
    assert_eq!(issue.quantity, 50);
    // All from the $25.00 layer: 50 × 2500 = 125000
    assert_eq!(issue.total_cost_minor, 125_000, "total cost = 50 × $25.00");
    assert_eq!(
        issue.consumed_layers.len(),
        1,
        "consumed from a single layer"
    );
    assert_eq!(issue.consumed_layers[0].quantity, 50);
    assert_eq!(issue.consumed_layers[0].unit_cost_minor, 2500);

    // --- Step 4: GL COGS posting — exactly-once ---
    let gl_payload = to_gl_payload(&issue, sku);
    let gl_event_id = issue.event_id;

    let entry_id =
        process_inventory_cogs_posting(&gl_pool, gl_event_id, &tenant_id, "inventory", &gl_payload)
            .await
            .expect("GL COGS posting must succeed");

    // Verify balanced journal entry: DR COGS $1250.00 / CR INVENTORY $1250.00
    let lines = get_journal_lines(&gl_pool, gl_event_id).await?;
    assert_eq!(lines.len(), 2, "exactly 2 journal lines");

    let cogs = lines.iter().find(|(a, _, _)| a == "COGS");
    let inv = lines.iter().find(|(a, _, _)| a == "INVENTORY");
    assert!(cogs.is_some(), "COGS line must exist");
    assert!(inv.is_some(), "INVENTORY line must exist");

    let (_, cogs_dr, cogs_cr) = cogs.unwrap();
    let (_, inv_dr, inv_cr) = inv.unwrap();

    // $1250.00 = 125000 minor units
    assert_eq!(*cogs_dr, 125_000, "COGS debit = $1250.00");
    assert_eq!(*cogs_cr, 0, "COGS credit = $0");
    assert_eq!(*inv_dr, 0, "INVENTORY debit = $0");
    assert_eq!(*inv_cr, 125_000, "INVENTORY credit = $1250.00");
    let _ = entry_id;

    // Entry is balanced
    assert_eq!(cogs_dr + inv_dr, cogs_cr + inv_cr, "entry must be balanced");

    // processed_events row exists
    let pe_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM processed_events WHERE event_id = $1")
            .bind(gl_event_id)
            .fetch_one(&gl_pool)
            .await?;
    assert_eq!(pe_count, 1, "exactly one processed_events row");

    // --- Step 5: Exactly-once — second GL post with same event_id → DuplicateEvent ---
    let second =
        process_inventory_cogs_posting(&gl_pool, gl_event_id, &tenant_id, "inventory", &gl_payload)
            .await;

    assert!(
        matches!(
            second,
            Err(gl_rs::services::journal_service::JournalError::DuplicateEvent(_))
        ),
        "second post with same event_id must return DuplicateEvent, got: {:?}",
        second,
    );

    // Still only one journal entry
    let entry_count = count_journal_entries(&gl_pool, gl_event_id).await?;
    assert_eq!(entry_count, 1, "no duplicate journal entries");

    cleanup_inventory(&inv_pool, &tenant_id).await;
    cleanup_gl(&gl_pool, &tenant_id).await;
    Ok(())
}

// ============================================================================
// Test 2: Period-close protection — GL posting rejected when period is closed
// ============================================================================

/// Verifies that a GL COGS posting is rejected when the accounting period
/// covering the issue date is closed.
///
/// Setup: create item, receive stock, issue stock, then construct a GL payload
/// with issued_at falling in a closed period (2024). Posting must fail.
#[tokio::test]
#[serial]
async fn inventory_gl_period_close_protection() -> Result<()> {
    let inv_pool = get_inventory_pool().await;
    let gl_pool = get_gl_pool().await;
    let tenant_id = generate_test_tenant();
    let sku = "SKU-CLOSED-PERIOD-001";

    // GL setup: closed period only (no open period for 2024)
    setup_gl_accounts(&gl_pool, &tenant_id).await?;
    setup_closed_period(&gl_pool, &tenant_id).await?;

    // Inventory: create item and receive stock
    let item = ItemRepo::create(
        &inv_pool,
        &CreateItemRequest {
            tenant_id: tenant_id.clone(),
            sku: sku.to_string(),
            name: "Closed Period Test Item".to_string(),
            description: None,
            inventory_account_ref: "INVENTORY".to_string(),
            cogs_account_ref: "COGS".to_string(),
            variance_account_ref: "COGS".to_string(),
            uom: None,
            tracking_mode: TrackingMode::None,
            make_buy: None,
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
            quantity: 100,
            unit_cost_minor: 1000,
            currency: "usd".to_string(),
            source_type: "purchase".to_string(),
            purchase_order_id: None,
            idempotency_key: format!("rcpt-cp-{}", Uuid::new_v4()),
            correlation_id: None,
            causation_id: None,
            lot_code: None,
            serial_codes: None,
            uom_id: None,
        },
        None,
    )
    .await
    .expect("receipt must succeed");

    let (issue, _) = process_issue(
        &inv_pool,
        &IssueRequest {
            tenant_id: tenant_id.clone(),
            item_id: item.id,
            warehouse_id,
            location_id: None,
            quantity: 10,
            currency: "usd".to_string(),
            source_module: "orders".to_string(),
            source_type: "sales_order".to_string(),
            source_id: "SO-CP-001".to_string(),
            source_line_id: None,
            idempotency_key: format!("issue-cp-{}", Uuid::new_v4()),
            correlation_id: None,
            causation_id: None,
            uom_id: None,
            lot_code: None,
            serial_codes: None,
        },
        None,
    )
    .await
    .expect("issue must succeed");

    // Construct GL payload with issued_at in the closed period (2024)
    let mut gl_payload = to_gl_payload(&issue, sku);
    gl_payload.issued_at = "2024-06-15T12:00:00Z"
        .parse::<chrono::DateTime<Utc>>()
        .expect("valid date");

    let result = process_inventory_cogs_posting(
        &gl_pool,
        Uuid::new_v4(), // fresh event_id — it's the period that should reject, not idempotency
        &tenant_id,
        "inventory",
        &gl_payload,
    )
    .await;

    assert!(
        result.is_err(),
        "GL posting into a closed period must fail, got Ok"
    );

    cleanup_inventory(&inv_pool, &tenant_id).await;
    cleanup_gl(&gl_pool, &tenant_id).await;
    Ok(())
}
