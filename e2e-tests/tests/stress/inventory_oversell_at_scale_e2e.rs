//! Stress test: Inventory oversell — 100 concurrent issues on 500 units prove conservation
//!
//! Proves that under 100 concurrent issue requests (each for 10 units) against
//! 500 units of stock, the conservation invariant holds: sum(issued) <= 500,
//! on-hand never goes negative, and rejected issues return clean business errors.
//!
//! The inventory module uses `SELECT … FOR UPDATE` (blocking) on FIFO layers,
//! so concurrent requests serialize at the row lock. Exactly 50 should succeed
//! (500 / 10 = 50), and 50 should fail with `InsufficientQuantity`.
//!
//! ## Running
//! ```bash
//! ./scripts/cargo-slot.sh test -p e2e-tests -- inventory_oversell_at_scale_e2e --nocapture
//! ```

use inventory_rs::domain::{
    issue_service::{process_issue, IssueError, IssueRequest},
    items::{CreateItemRequest, ItemRepo, TrackingMode},
    receipt_service::{process_receipt, ReceiptRequest},
};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

const CONCURRENCY: usize = 100;
const INITIAL_STOCK: i64 = 500;
const ISSUE_QTY: i64 = 10;

async fn get_inventory_pool() -> PgPool {
    let url = std::env::var("INVENTORY_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| {
            "postgresql://inventory_user:inventory_pass@localhost:5442/inventory_db".to_string()
        });
    let pool = PgPoolOptions::new()
        .max_connections(20)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&url)
        .await
        .expect("failed to connect to inventory DB");

    sqlx::migrate!("../modules/inventory/db/migrations")
        .run(&pool)
        .await
        .expect("failed to run inventory migrations");

    pool
}

async fn cleanup_tenant(pool: &PgPool, tenant_id: &str) {
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

#[derive(Debug)]
struct IssueOutcome {
    issued_qty: i64,
    is_insufficient: bool,
    is_unexpected_error: bool,
    error_msg: Option<String>,
}

#[tokio::test]
async fn inventory_oversell_at_scale_e2e() {
    let pool = Arc::new(get_inventory_pool().await);
    let tenant_id = Arc::new(format!("stress-inv-{}", Uuid::new_v4()));

    // --- Seed: create item ---
    let item = ItemRepo::create(
        &pool,
        &CreateItemRequest {
            tenant_id: (*tenant_id).clone(),
            sku: format!("STRESS-{}", Uuid::new_v4()),
            name: "Stress Test Oversell Item".to_string(),
            description: None,
            inventory_account_ref: "1200".to_string(),
            cogs_account_ref: "5000".to_string(),
            variance_account_ref: "5010".to_string(),
            uom: None,
            tracking_mode: TrackingMode::None,
            make_buy: None,
        },
    )
    .await
    .expect("failed to create item");

    let item_id = item.id;
    let warehouse_id = Uuid::new_v4();

    // --- Seed: receive 500 units @ $10.00 ---
    process_receipt(
        &pool,
        &ReceiptRequest {
            tenant_id: (*tenant_id).clone(),
            item_id,
            warehouse_id,
            location_id: None,
            quantity: INITIAL_STOCK,
            unit_cost_minor: 1000, // $10.00
            currency: "usd".to_string(),
            source_type: "purchase".to_string(),
            purchase_order_id: None,
            idempotency_key: format!("stress-rcpt-{}", Uuid::new_v4()),
            correlation_id: None,
            causation_id: None,
            lot_code: None,
            serial_codes: None,
            uom_id: None,
        },
        None,
    )
    .await
    .expect("failed to seed stock");

    println!(
        "seeded: tenant={}, item={}, warehouse={}, stock={}",
        tenant_id, item_id, warehouse_id, INITIAL_STOCK
    );

    // --- Fire 100 concurrent issue requests, each for 10 units ---
    // Total attempted = 1000 > 500 available. Exactly 50 should succeed.
    println!(
        "\n--- {} concurrent issues of {} units (total attempted: {}, stock: {}) ---",
        CONCURRENCY,
        ISSUE_QTY,
        CONCURRENCY as i64 * ISSUE_QTY,
        INITIAL_STOCK
    );

    let start = Instant::now();

    let handles: Vec<_> = (0..CONCURRENCY)
        .map(|i| {
            let pool = Arc::clone(&pool);
            let tenant_id = Arc::clone(&tenant_id);
            tokio::spawn(async move {
                match process_issue(
                    &pool,
                    &IssueRequest {
                        tenant_id: (*tenant_id).clone(),
                        item_id,
                        warehouse_id,
                        location_id: None,
                        quantity: ISSUE_QTY,
                        currency: "usd".to_string(),
                        source_module: "stress-test".to_string(),
                        source_type: "sales_order".to_string(),
                        source_id: format!("STRESS-SO-{:03}", i),
                        source_line_id: None,
                        idempotency_key: format!("stress-issue-{}-{}", i, Uuid::new_v4()),
                        correlation_id: Some(format!("stress-{}", i)),
                        causation_id: None,
                        uom_id: None,
                        lot_code: None,
                        serial_codes: None,
                    },
                    None,
                )
                .await
                {
                    Ok((result, _is_replay)) => IssueOutcome {
                        issued_qty: result.quantity,
                        is_insufficient: false,
                        is_unexpected_error: false,
                        error_msg: None,
                    },
                    Err(IssueError::InsufficientQuantity { .. }) => IssueOutcome {
                        issued_qty: 0,
                        is_insufficient: true,
                        is_unexpected_error: false,
                        error_msg: None,
                    },
                    Err(IssueError::NoLayersAvailable) => IssueOutcome {
                        issued_qty: 0,
                        is_insufficient: true,
                        is_unexpected_error: false,
                        error_msg: None,
                    },
                    Err(e) => IssueOutcome {
                        issued_qty: 0,
                        is_insufficient: false,
                        is_unexpected_error: true,
                        error_msg: Some(format!("{}", e)),
                    },
                }
            })
        })
        .collect();

    let mut outcomes = Vec::with_capacity(CONCURRENCY);
    for h in handles {
        outcomes.push(h.await.expect("task panicked"));
    }
    let elapsed = start.elapsed();

    // --- Analyze results ---
    let total_issued: i64 = outcomes.iter().map(|o| o.issued_qty).sum();
    let success_count = outcomes.iter().filter(|o| o.issued_qty > 0).count();
    let insufficient_count = outcomes.iter().filter(|o| o.is_insufficient).count();
    let unexpected_error_count = outcomes.iter().filter(|o| o.is_unexpected_error).count();

    println!("completed in {:?}", elapsed);
    println!("  successful issues: {} (issued > 0)", success_count);
    println!("  insufficient stock rejections: {}", insufficient_count);
    println!("  unexpected errors: {}", unexpected_error_count);
    println!("  total issued from responses: {} units", total_issued);

    for (i, o) in outcomes.iter().enumerate() {
        if o.is_unexpected_error {
            println!(
                "  request {}: UNEXPECTED ERROR: {}",
                i,
                o.error_msg.as_deref().unwrap_or("unknown")
            );
        }
    }

    // --- Assertion 1: No unexpected errors ---
    assert_eq!(
        unexpected_error_count, 0,
        "no unexpected errors expected — all rejections should be clean InsufficientQuantity"
    );

    // --- Assertion 2: Conservation invariant (response-level) ---
    assert!(
        total_issued <= INITIAL_STOCK,
        "CONSERVATION VIOLATION (responses): total issued {} exceeds initial stock {}",
        total_issued,
        INITIAL_STOCK
    );

    // --- Assertion 3: Exactly 50 succeeded (500 / 10) ---
    let expected_successes = (INITIAL_STOCK / ISSUE_QTY) as usize;
    assert_eq!(
        success_count, expected_successes,
        "expected exactly {} successful issues (stock {} / qty {}), got {}",
        expected_successes, INITIAL_STOCK, ISSUE_QTY, success_count
    );

    // --- Assertion 4: All others rejected cleanly ---
    assert_eq!(
        insufficient_count,
        CONCURRENCY - expected_successes,
        "expected {} insufficient stock rejections, got {}",
        CONCURRENCY - expected_successes,
        insufficient_count
    );

    // --- Assertion 5: Total issued == initial stock (exact drain) ---
    assert_eq!(
        total_issued, INITIAL_STOCK,
        "total issued ({}) must equal initial stock ({}) — exact drain",
        total_issued, INITIAL_STOCK
    );

    // --- Assertion 6: DB-level conservation — on-hand projection is 0 ---
    let db_on_hand: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(quantity_remaining), 0)::BIGINT FROM inventory_layers WHERE tenant_id = $1 AND item_id = $2 AND warehouse_id = $3",
    )
    .bind(tenant_id.as_ref())
    .bind(item_id)
    .bind(warehouse_id)
    .fetch_one(pool.as_ref())
    .await
    .expect("failed to query on-hand");

    println!("\n  DB on-hand (layer sum): {} units", db_on_hand);

    assert!(
        db_on_hand >= 0,
        "on-hand must never be negative, got {}",
        db_on_hand
    );

    assert_eq!(
        db_on_hand, 0,
        "on-hand must be 0 after full drain, got {}",
        db_on_hand
    );

    // --- Assertion 7: DB ledger row count matches successes ---
    let ledger_issue_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::BIGINT FROM inventory_ledger WHERE tenant_id = $1 AND item_id = $2 AND entry_type = 'issued'",
    )
    .bind(tenant_id.as_ref())
    .bind(item_id)
    .fetch_one(pool.as_ref())
    .await
    .expect("failed to query ledger");

    assert_eq!(
        ledger_issue_count, expected_successes as i64,
        "ledger issue rows ({}) must match successful issues ({})",
        ledger_issue_count, expected_successes
    );

    println!("  ledger issue rows: {}", ledger_issue_count);
    println!("  on-hand non-negative: YES");
    println!("  conservation invariant: PASSED");

    cleanup_tenant(pool.as_ref(), &tenant_id).await;
}
