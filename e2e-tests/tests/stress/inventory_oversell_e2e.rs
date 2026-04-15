//! Stress test: Inventory oversell — 50 concurrent reservations prove stock limits
//!
//! Proves that under 50 concurrent reservation requests (each for 1 unit) against
//! 10 units of stock, the conservation invariant holds: exactly 10 reservations
//! succeed, 40 are rejected with insufficient stock, quantity_available ends at 0,
//! and quantity_on_hand remains unchanged (reservations hold, not consume).
//!
//! The reservation service must check available stock under row-level locking
//! (SELECT … FOR UPDATE on item_on_hand) so concurrent requests serialize at the
//! lock. This prevents the classic oversell: two transactions both read available=1,
//! both succeed, driving available negative.
//!
//! ## Running
//! ```bash
//! ./scripts/cargo-slot.sh test -p e2e-tests -- inventory_oversell_e2e --nocapture
//! ```

use inventory_rs::domain::{
    items::{CreateItemRequest, ItemRepo, TrackingMode},
    receipt_service::{process_receipt, ReceiptRequest},
    reservation_service::{process_reserve, ReservationError, ReserveRequest},
};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

const CONCURRENCY: usize = 50;
const INITIAL_STOCK: i64 = 10;
const RESERVE_QTY: i64 = 1;

async fn get_inventory_pool() -> PgPool {
    let url = std::env::var("INVENTORY_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| {
            "postgresql://inventory_user:inventory_pass@localhost:5442/inventory_db".to_string()
        });
    let pool = PgPoolOptions::new()
        .max_connections(100)
        .acquire_timeout(Duration::from_secs(30))
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
struct ReserveOutcome {
    reserved_qty: i64,
    is_insufficient: bool,
    is_unexpected_error: bool,
    error_msg: Option<String>,
}

#[tokio::test]
async fn inventory_oversell_e2e() {
    let pool = Arc::new(get_inventory_pool().await);
    let tenant_id = Arc::new(format!("stress-rsv-{}", Uuid::new_v4()));

    // --- Phase 1: Seed — create item and receive 10 units ---
    let item = ItemRepo::create(
        &pool,
        &CreateItemRequest {
            tenant_id: (*tenant_id).clone(),
            sku: format!("STRESS-RSV-{}", Uuid::new_v4()),
            name: "Stress Test Oversell Reservation Item".to_string(),
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

    // --- Phase 2: Burst — fire 50 concurrent reservations of 1 unit each ---
    // Total attempted = 50 > 10 available. Exactly 10 should succeed.
    println!(
        "\n--- {} concurrent reservations of {} unit(s) (total attempted: {}, stock: {}) ---",
        CONCURRENCY,
        RESERVE_QTY,
        CONCURRENCY as i64 * RESERVE_QTY,
        INITIAL_STOCK
    );

    let start = Instant::now();

    let handles: Vec<_> = (0..CONCURRENCY)
        .map(|i| {
            let pool = Arc::clone(&pool);
            let tenant_id = Arc::clone(&tenant_id);
            tokio::spawn(async move {
                match process_reserve(
                    &pool,
                    &ReserveRequest {
                        tenant_id: (*tenant_id).clone(),
                        item_id,
                        warehouse_id,
                        quantity: RESERVE_QTY,
                        reference_type: Some("sales_order".to_string()),
                        reference_id: Some(format!("STRESS-SO-{:03}", i)),
                        expires_at: None,
                        idempotency_key: format!("stress-rsv-{}-{}", i, Uuid::new_v4()),
                        correlation_id: Some(format!("stress-{}", i)),
                        causation_id: None,
                    },
                )
                .await
                {
                    Ok((result, _is_replay)) => ReserveOutcome {
                        reserved_qty: result.quantity,
                        is_insufficient: false,
                        is_unexpected_error: false,
                        error_msg: None,
                    },
                    Err(ReservationError::InsufficientAvailable { .. }) => ReserveOutcome {
                        reserved_qty: 0,
                        is_insufficient: true,
                        is_unexpected_error: false,
                        error_msg: None,
                    },
                    Err(e) => ReserveOutcome {
                        reserved_qty: 0,
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

    // --- Phase 3: Analyze results ---
    let total_reserved: i64 = outcomes.iter().map(|o| o.reserved_qty).sum();
    let success_count = outcomes.iter().filter(|o| o.reserved_qty > 0).count();
    let insufficient_count = outcomes.iter().filter(|o| o.is_insufficient).count();
    let unexpected_error_count = outcomes.iter().filter(|o| o.is_unexpected_error).count();

    println!("completed in {:?}", elapsed);
    println!(
        "  successful reservations: {} (reserved > 0)",
        success_count
    );
    println!("  insufficient stock rejections: {}", insufficient_count);
    println!("  unexpected errors: {}", unexpected_error_count);
    println!("  total reserved from responses: {} units", total_reserved);

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
        "no unexpected errors expected — all rejections should be clean insufficient-stock errors"
    );

    // --- Assertion 2: Conservation invariant (response-level) ---
    assert!(
        total_reserved <= INITIAL_STOCK,
        "CONSERVATION VIOLATION (responses): total reserved {} exceeds initial stock {}",
        total_reserved,
        INITIAL_STOCK
    );

    // --- Assertion 3: Exactly 10 succeeded (10 / 1) ---
    let expected_successes = (INITIAL_STOCK / RESERVE_QTY) as usize;
    assert_eq!(
        success_count, expected_successes,
        "expected exactly {} successful reservations (stock {} / qty {}), got {}",
        expected_successes, INITIAL_STOCK, RESERVE_QTY, success_count
    );

    // --- Assertion 4: All others rejected cleanly ---
    assert_eq!(
        insufficient_count,
        CONCURRENCY - expected_successes,
        "expected {} insufficient stock rejections, got {}",
        CONCURRENCY - expected_successes,
        insufficient_count
    );

    // --- Assertion 5: Total reserved == initial stock (exact drain of available) ---
    assert_eq!(
        total_reserved, INITIAL_STOCK,
        "total reserved ({}) must equal initial stock ({}) — exact drain",
        total_reserved, INITIAL_STOCK
    );

    // --- Assertion 6: DB-level conservation — quantity_available is 0 ---
    let db_available: i64 = sqlx::query_scalar(
        "SELECT COALESCE(quantity_available, 0)::BIGINT FROM item_on_hand WHERE tenant_id = $1 AND item_id = $2 AND warehouse_id = $3 AND location_id IS NULL",
    )
    .bind(tenant_id.as_ref())
    .bind(item_id)
    .bind(warehouse_id)
    .fetch_one(pool.as_ref())
    .await
    .expect("failed to query quantity_available");

    println!("\n  DB quantity_available: {} units", db_available);

    assert!(
        db_available >= 0,
        "quantity_available must never be negative, got {}",
        db_available
    );

    assert_eq!(
        db_available, 0,
        "quantity_available must be 0 after full reservation drain, got {}",
        db_available
    );

    // --- Assertion 7: DB reservation row count matches successes ---
    let db_reservation_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::BIGINT FROM inventory_reservations WHERE tenant_id = $1 AND item_id = $2 AND status = 'active' AND reverses_reservation_id IS NULL",
    )
    .bind(tenant_id.as_ref())
    .bind(item_id)
    .fetch_one(pool.as_ref())
    .await
    .expect("failed to query reservations");

    assert_eq!(
        db_reservation_count, expected_successes as i64,
        "active reservation rows ({}) must match successful reservations ({})",
        db_reservation_count, expected_successes
    );

    // --- Assertion 8: quantity_on_hand unchanged (reservations hold, not consume) ---
    let db_on_hand: i64 = sqlx::query_scalar(
        "SELECT COALESCE(quantity_on_hand, 0)::BIGINT FROM item_on_hand WHERE tenant_id = $1 AND item_id = $2 AND warehouse_id = $3 AND location_id IS NULL",
    )
    .bind(tenant_id.as_ref())
    .bind(item_id)
    .bind(warehouse_id)
    .fetch_one(pool.as_ref())
    .await
    .expect("failed to query quantity_on_hand");

    assert_eq!(
        db_on_hand, INITIAL_STOCK,
        "quantity_on_hand must remain {} (reservations hold, not consume), got {}",
        INITIAL_STOCK, db_on_hand
    );

    // --- Post-burst health check ---
    let health_check: (i64,) = sqlx::query_as("SELECT 1::BIGINT")
        .fetch_one(pool.as_ref())
        .await
        .expect("post-burst health check FAILED — DB pool unhealthy");
    assert_eq!(health_check.0, 1);

    println!("  reservation rows (active): {}", db_reservation_count);
    println!("  quantity_on_hand: {} (unchanged)", db_on_hand);
    println!("  quantity_available: {} (fully reserved)", db_available);
    println!("  on-hand non-negative: YES");
    println!("  conservation invariant: PASSED");
    println!("  post-burst health check: PASSED");

    cleanup_tenant(pool.as_ref(), &tenant_id).await;
}
