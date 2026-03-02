//! E2E Test: Inventory 10-Way Concurrency — No Oversell, Consistent Costing (bd-2x3v)
//!
//! ## Coverage
//! 1. 10 concurrent issue attempts on limited stock (50 units):
//!    - Each issues 10 units. Stock covers exactly 5 issues.
//!    - FIFO SELECT … FOR UPDATE serializes the lock; no oversell.
//!    - Exactly 5 succeed, 5 fail with InsufficientQuantity.
//!    - Total issued ≤ 50 (conservation invariant).
//! 2. All successful issues have consistent FIFO costing:
//!    - unit_cost_minor in consumed_layers matches the single receipt layer's cost.
//! 3. After all concurrent issues, on-hand projection is exactly 0.
//!
//! ## Pattern
//! No Docker, no mocks — live inventory DB.
//! Uses tokio::spawn for true async concurrency.

use inventory_rs::domain::{
    issue_service::{process_issue, IssueError, IssueRequest},
    items::{CreateItemRequest, ItemRepo, TrackingMode},
    receipt_service::{process_receipt, ReceiptRequest},
};
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use uuid::Uuid;

// ============================================================================
// Helpers
// ============================================================================

async fn get_inventory_pool() -> sqlx::PgPool {
    let url = std::env::var("INVENTORY_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| {
            "postgresql://inventory_user:inventory_pass@localhost:5442/inventory_db".to_string()
        });

    let pool = PgPoolOptions::new()
        .max_connections(20) // extra headroom for concurrent tasks
        .connect(&url)
        .await
        .expect("Failed to connect to inventory DB — is INVENTORY_DATABASE_URL set?");

    sqlx::migrate!("../modules/inventory/db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run inventory migrations");

    pool
}

async fn cleanup_tenant(pool: &sqlx::PgPool, tenant_id: &str) {
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

// ============================================================================
// Test 1: 10-way concurrent issue — no oversell
// ============================================================================

/// 10 goroutines each attempt to issue 10 units from 50-unit stock.
///
/// Stock covers 5 issues exactly (5 × 10 = 50).
/// FIFO row locking serializes access: exactly 5 succeed, 5 fail.
/// Total issued quantity is always exactly 50.
#[tokio::test]
async fn inventory_concurrency_no_oversell_10_way() {
    let pool = Arc::new(get_inventory_pool().await);
    let tenant_id = Arc::new(format!("e2e-{}", Uuid::new_v4()));

    // Create item
    let item = ItemRepo::create(
        &pool,
        &CreateItemRequest {
            tenant_id: (*tenant_id).clone(),
            sku: "E2E-CONC-001".to_string(),
            name: "Concurrency Test Item".to_string(),
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

    let item_id = item.id;
    let warehouse_id = Uuid::new_v4();

    // Receive exactly 50 units @ $10.00
    process_receipt(
        &pool,
        &ReceiptRequest {
            tenant_id: (*tenant_id).clone(),
            item_id,
            warehouse_id,
            location_id: None,
            quantity: 50,
            unit_cost_minor: 1000, // $10.00
            currency: "usd".to_string(),
            purchase_order_id: None,
            idempotency_key: format!("rcpt-conc-{}", Uuid::new_v4()),
            correlation_id: None,
            causation_id: None,
            lot_code: None,
            serial_codes: None,
            uom_id: None,
        },
    )
    .await
    .expect("seed stock");

    // Spawn 10 concurrent issue tasks, each requesting 10 units
    let mut handles = Vec::with_capacity(10);
    for i in 0..10usize {
        let pool = Arc::clone(&pool);
        let tenant_id = Arc::clone(&tenant_id);
        let idempotency_key = format!("issue-conc-{}-{}", i, Uuid::new_v4());

        let handle = tokio::spawn(async move {
            process_issue(
                &pool,
                &IssueRequest {
                    tenant_id: (*tenant_id).clone(),
                    item_id,
                    warehouse_id,
                    location_id: None,
                    quantity: 10,
                    currency: "usd".to_string(),
                    source_module: "orders".to_string(),
                    source_type: "sales_order".to_string(),
                    source_id: format!("SO-CONC-{:03}", i),
                    source_line_id: None,
                    idempotency_key,
                    correlation_id: Some(format!("e2e-conc-{}", i)),
                    causation_id: None,
                    uom_id: None,
                    lot_code: None,
                    serial_codes: None,
                },
            )
            .await
        });
        handles.push(handle);
    }

    // Collect results
    let mut successes = 0usize;
    let mut failures = 0usize;
    let mut total_issued: i64 = 0;

    for handle in handles {
        match handle.await.expect("task did not panic") {
            Ok((result, _is_replay)) => {
                successes += 1;
                total_issued += result.quantity;
                // Verify each success has consistent unit cost
                for cl in &result.consumed_layers {
                    assert_eq!(
                        cl.unit_cost_minor, 1000,
                        "all consumed layers must have $10.00 unit cost"
                    );
                }
            }
            Err(IssueError::InsufficientQuantity { .. }) => {
                failures += 1;
            }
            Err(e) => {
                panic!("unexpected error (not InsufficientQuantity): {:?}", e);
            }
        }
    }

    // Exactly 5 succeed, 5 fail
    assert_eq!(
        successes, 5,
        "exactly 5 of 10 concurrent issues must succeed (stock covers 5)"
    );
    assert_eq!(failures, 5, "exactly 5 must fail with InsufficientQuantity");

    // Conservation: total issued == initial stock
    assert_eq!(
        total_issued, 50,
        "total issued ({}) must equal initial stock (50) — no oversell",
        total_issued
    );

    // On-hand projection after full drain
    let on_hand: i64 = sqlx::query_scalar(
        "SELECT quantity_on_hand FROM item_on_hand WHERE tenant_id = $1 AND item_id = $2 AND warehouse_id = $3",
    )
    .bind((*tenant_id).as_str())
    .bind(item_id)
    .bind(warehouse_id)
    .fetch_one(&*pool)
    .await
    .expect("on-hand row");
    assert_eq!(on_hand, 0, "on-hand must be 0 after full drain");

    // Layer fully consumed
    let layer_remaining: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(quantity_remaining), 0)::bigint FROM inventory_layers WHERE tenant_id = $1 AND item_id = $2",
    )
    .bind((*tenant_id).as_str())
    .bind(item_id)
    .fetch_one(&*pool)
    .await
    .expect("layer remaining sum");
    assert_eq!(layer_remaining, 0, "all layers fully consumed");

    cleanup_tenant(&pool, &tenant_id).await;
}

// ============================================================================
// Test 2: 10-way concurrency with partial stock — first-in wins
// ============================================================================

/// 10 concurrent issues each requesting 15 units from 80 units of stock.
/// Only floor(80/15) = 5 full issues possible, with a 5-unit remainder.
/// Verifies:
/// - total_issued ≤ 80 (conservation)
/// - no issue reports issued > 15 (correct quantity)
/// - on-hand at the end equals initial_stock - total_issued
#[tokio::test]
async fn inventory_concurrency_conservation_invariant() {
    let pool = Arc::new(get_inventory_pool().await);
    let tenant_id = Arc::new(format!("e2e-{}", Uuid::new_v4()));

    let item = ItemRepo::create(
        &pool,
        &CreateItemRequest {
            tenant_id: (*tenant_id).clone(),
            sku: "E2E-CONC-002".to_string(),
            name: "Concurrency Conservation Item".to_string(),
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

    let item_id = item.id;
    let warehouse_id = Uuid::new_v4();
    let initial_stock: i64 = 80;
    let issue_qty: i64 = 15;

    process_receipt(
        &pool,
        &ReceiptRequest {
            tenant_id: (*tenant_id).clone(),
            item_id,
            warehouse_id,
            location_id: None,
            quantity: initial_stock,
            unit_cost_minor: 500, // $5.00
            currency: "usd".to_string(),
            purchase_order_id: None,
            idempotency_key: format!("rcpt-conc2-{}", Uuid::new_v4()),
            correlation_id: None,
            causation_id: None,
            lot_code: None,
            serial_codes: None,
            uom_id: None,
        },
    )
    .await
    .expect("seed stock");

    let mut handles = Vec::with_capacity(10);
    for i in 0..10usize {
        let pool = Arc::clone(&pool);
        let tenant_id = Arc::clone(&tenant_id);

        let handle = tokio::spawn(async move {
            process_issue(
                &pool,
                &IssueRequest {
                    tenant_id: (*tenant_id).clone(),
                    item_id,
                    warehouse_id,
                    location_id: None,
                    quantity: issue_qty,
                    currency: "usd".to_string(),
                    source_module: "orders".to_string(),
                    source_type: "sales_order".to_string(),
                    source_id: format!("SO-CONC2-{:03}", i),
                    source_line_id: None,
                    idempotency_key: format!("issue-conc2-{}-{}", i, Uuid::new_v4()),
                    correlation_id: None,
                    causation_id: None,
                    uom_id: None,
                    lot_code: None,
                    serial_codes: None,
                },
            )
            .await
        });
        handles.push(handle);
    }

    let mut total_issued: i64 = 0;
    for handle in handles {
        match handle.await.expect("task did not panic") {
            Ok((result, _)) => {
                assert!(
                    result.quantity <= issue_qty,
                    "issued qty must not exceed requested"
                );
                total_issued += result.quantity;
            }
            Err(IssueError::InsufficientQuantity { .. }) => {
                // Expected for some
            }
            Err(e) => panic!("unexpected error: {:?}", e),
        }
    }

    // Conservation invariant: total issued ≤ initial stock
    assert!(
        total_issued <= initial_stock,
        "total issued ({}) must not exceed initial stock ({}) — no oversell",
        total_issued,
        initial_stock
    );

    // On-hand = initial - issued
    let on_hand: i64 = sqlx::query_scalar(
        "SELECT quantity_on_hand FROM item_on_hand WHERE tenant_id = $1 AND item_id = $2 AND warehouse_id = $3",
    )
    .bind((*tenant_id).as_str())
    .bind(item_id)
    .bind(warehouse_id)
    .fetch_one(&*pool)
    .await
    .expect("on-hand row");
    assert_eq!(
        on_hand,
        initial_stock - total_issued,
        "on-hand must equal initial_stock - total_issued"
    );

    cleanup_tenant(&pool, &tenant_id).await;
}
