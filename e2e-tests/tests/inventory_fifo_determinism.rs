//! E2E Test: Inventory FIFO Determinism (bd-2x3v)
//!
//! ## Coverage
//! 1. Two-layer FIFO — oldest layer consumed first, then next.
//!    Cost is deterministic: consumed_layers match expected layer IDs and costs.
//! 2. Three-layer FIFO spanning all layers — verified by individual layer breakdown.
//! 3. Partial consumption — issue spanning exactly into second layer.
//! 4. Cost sum invariant: sum(consumed_layers.extended_cost_minor) == total_cost_minor.
//!
//! ## Key Invariants
//! FIFO order: oldest receipt first (received_at ASC, ledger_entry_id ASC).
//! Costs are exact integer arithmetic — no floating point, no rounding.
//!
//! ## Pattern
//! No Docker, no mocks — uses live inventory DB.

use inventory_rs::domain::{
    issue_service::{process_issue, IssueRequest},
    items::{CreateItemRequest, ItemRepo, TrackingMode},
    receipt_service::{process_receipt, ReceiptRequest},
};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
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
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to inventory DB — is INVENTORY_DATABASE_URL set?");

    sqlx::migrate!("../modules/inventory/db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run inventory migrations");

    pool
}

fn item_req(tenant_id: &str, sku: &str) -> CreateItemRequest {
    CreateItemRequest {
        tenant_id: tenant_id.to_string(),
        sku: sku.to_string(),
        name: "FIFO Determinism Item".to_string(),
        description: None,
        inventory_account_ref: "1200".to_string(),
        cogs_account_ref: "5000".to_string(),
        variance_account_ref: "5010".to_string(),
        uom: None,
        tracking_mode: TrackingMode::None,
        make_buy: None,
    }
}

fn receipt_req(
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
    qty: i64,
    cost: i64,
) -> ReceiptRequest {
    ReceiptRequest {
        tenant_id: tenant_id.to_string(),
        item_id,
        warehouse_id,
        location_id: None,
        quantity: qty,
        unit_cost_minor: cost,
        currency: "usd".to_string(),
        source_type: "purchase".to_string(),
        purchase_order_id: None,
        idempotency_key: format!("rcpt-fifo-{}", Uuid::new_v4()),
        correlation_id: Some("e2e-fifo".to_string()),
        causation_id: None,
        lot_code: None,
        serial_codes: None,
        uom_id: None,
    }
}

fn issue_req(tenant_id: &str, item_id: Uuid, warehouse_id: Uuid, qty: i64) -> IssueRequest {
    IssueRequest {
        tenant_id: tenant_id.to_string(),
        item_id,
        warehouse_id,
        location_id: None,
        quantity: qty,
        currency: "usd".to_string(),
        source_module: "orders".to_string(),
        source_type: "sales_order".to_string(),
        source_id: format!("SO-FIFO-{}", Uuid::new_v4()),
        source_line_id: None,
        idempotency_key: format!("issue-fifo-{}", Uuid::new_v4()),
        correlation_id: Some("e2e-fifo".to_string()),
        causation_id: None,
        uom_id: None,
        lot_code: None,
        serial_codes: None,
    }
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
        sqlx::query(q)
            .bind(tenant_id)
            .execute(pool)
            .await
            .ok();
    }
}

// ============================================================================
// Test 1: Two-layer FIFO — oldest consumed first
// ============================================================================

/// Receives two layers at different costs, then issues spanning into the second.
///
/// Layer A: 30 units @ $10.00 (received first)
/// Layer B: 40 units @ $20.00 (received second)
///
/// Issue 40 units:
///   → consume all 30 from Layer A ($300.00 = 30000 minor)
///   → consume 10 from Layer B ($200.00 = 20000 minor)
///   → total cost = $500.00 = 50000 minor
///
/// FIFO invariant: Layer A must appear first in consumed_layers.
#[tokio::test]
#[serial]
async fn inventory_fifo_two_layers_consumed_oldest_first() {
    let pool = get_inventory_pool().await;
    let tenant_id = format!("e2e-{}", Uuid::new_v4());

    let item = ItemRepo::create(&pool, &item_req(&tenant_id, "E2E-FIFO-2L-001"))
        .await
        .expect("create item");
    let warehouse_id = Uuid::new_v4();

    // Layer A: 30 units @ $10.00
    let (rcpt_a, _) = process_receipt(
        &pool,
        &receipt_req(&tenant_id, item.id, warehouse_id, 30, 1000),
        None,
    )
    .await
    .expect("Layer A receipt");

    // Small sleep ensures Layer B has a later timestamp even on fast machines.
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    // Layer B: 40 units @ $20.00
    let (rcpt_b, _) = process_receipt(
        &pool,
        &receipt_req(&tenant_id, item.id, warehouse_id, 40, 2000),
        None,
    )
    .await
    .expect("Layer B receipt");

    // Issue 40 units — must consume 30 from A then 10 from B
    let (issue, is_replay) = process_issue(
        &pool,
        &issue_req(&tenant_id, item.id, warehouse_id, 40),
        None,
    )
    .await
    .expect("issue must succeed");
    assert!(!is_replay);
    assert_eq!(issue.quantity, 40);

    // Total cost: 30×1000 + 10×2000 = 30000 + 20000 = 50000
    assert_eq!(
        issue.total_cost_minor, 50_000,
        "total cost must be 30×$10 + 10×$20 = $500.00"
    );

    // Cost sum invariant
    let sum: i64 = issue
        .consumed_layers
        .iter()
        .map(|cl| cl.extended_cost_minor)
        .sum();
    assert_eq!(
        sum, issue.total_cost_minor,
        "sum(extended_cost_minor) must equal total_cost_minor"
    );

    // FIFO order: Layer A first
    assert!(
        issue.consumed_layers.len() >= 2,
        "must have consumed from at least 2 layers"
    );
    assert_eq!(
        issue.consumed_layers[0].layer_id, rcpt_a.layer_id,
        "oldest layer (A) must be consumed first"
    );
    assert_eq!(
        issue.consumed_layers[0].quantity, 30,
        "Layer A: consume all 30"
    );
    assert_eq!(
        issue.consumed_layers[0].unit_cost_minor, 1000,
        "Layer A cost = $10.00"
    );

    assert_eq!(
        issue.consumed_layers[1].layer_id, rcpt_b.layer_id,
        "Layer B consumed second"
    );
    assert_eq!(issue.consumed_layers[1].quantity, 10, "Layer B: consume 10");
    assert_eq!(
        issue.consumed_layers[1].unit_cost_minor, 2000,
        "Layer B cost = $20.00"
    );

    // Verify remaining quantities in DB
    let layer_a_rem: i64 =
        sqlx::query_scalar("SELECT quantity_remaining FROM inventory_layers WHERE id = $1")
            .bind(rcpt_a.layer_id)
            .fetch_one(&pool)
            .await
            .expect("layer A remaining");
    assert_eq!(layer_a_rem, 0, "Layer A fully consumed");

    let layer_b_rem: i64 =
        sqlx::query_scalar("SELECT quantity_remaining FROM inventory_layers WHERE id = $1")
            .bind(rcpt_b.layer_id)
            .fetch_one(&pool)
            .await
            .expect("layer B remaining");
    assert_eq!(layer_b_rem, 30, "Layer B: 30 remaining (40 - 10 = 30)");

    cleanup_tenant(&pool, &tenant_id).await;
}

// ============================================================================
// Test 2: Three-layer FIFO — full consumption breakdown verified
// ============================================================================

/// Receives three layers, issues spanning all three.
///
/// Layer A: 10 units @ $5.00  (cost = $50.00  = 5000 minor)
/// Layer B: 20 units @ $8.00  (cost = $160.00 = 16000 minor)
/// Layer C: 30 units @ $12.00 (cost = $360.00 = 36000 minor)
///
/// Issue 60 units (all stock):
///   → consume 10 from A ($50.00)
///   → consume 20 from B ($160.00)
///   → consume 30 from C ($360.00)
///   → total = $570.00 = 57000 minor
#[tokio::test]
#[serial]
async fn inventory_fifo_three_layers_full_consumption() {
    let pool = get_inventory_pool().await;
    let tenant_id = format!("e2e-{}", Uuid::new_v4());

    let item = ItemRepo::create(&pool, &item_req(&tenant_id, "E2E-FIFO-3L-001"))
        .await
        .expect("create item");
    let warehouse_id = Uuid::new_v4();

    // Receive 3 layers with increasing timestamps
    let (rcpt_a, _) = process_receipt(
        &pool,
        &receipt_req(&tenant_id, item.id, warehouse_id, 10, 500),
        None,
    )
    .await
    .expect("Layer A");
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    let (rcpt_b, _) = process_receipt(
        &pool,
        &receipt_req(&tenant_id, item.id, warehouse_id, 20, 800),
        None,
    )
    .await
    .expect("Layer B");
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    let (rcpt_c, _) = process_receipt(
        &pool,
        &receipt_req(&tenant_id, item.id, warehouse_id, 30, 1200),
        None,
    )
    .await
    .expect("Layer C");

    // Issue all 60 units
    let (issue, _) = process_issue(
        &pool,
        &issue_req(&tenant_id, item.id, warehouse_id, 60),
        None,
    )
    .await
    .expect("issue must succeed");

    assert_eq!(issue.quantity, 60);
    // 10×500 + 20×800 + 30×1200 = 5000 + 16000 + 36000 = 57000
    assert_eq!(issue.total_cost_minor, 57_000, "total cost = $570.00");

    // Cost sum invariant
    let sum: i64 = issue
        .consumed_layers
        .iter()
        .map(|cl| cl.extended_cost_minor)
        .sum();
    assert_eq!(sum, 57_000, "sum(extended_cost_minor) == total_cost_minor");

    // Exactly 3 consumed layers in FIFO order
    assert_eq!(issue.consumed_layers.len(), 3, "3 consumed layers");
    assert_eq!(issue.consumed_layers[0].layer_id, rcpt_a.layer_id);
    assert_eq!(issue.consumed_layers[0].quantity, 10);
    assert_eq!(issue.consumed_layers[0].extended_cost_minor, 5_000);

    assert_eq!(issue.consumed_layers[1].layer_id, rcpt_b.layer_id);
    assert_eq!(issue.consumed_layers[1].quantity, 20);
    assert_eq!(issue.consumed_layers[1].extended_cost_minor, 16_000);

    assert_eq!(issue.consumed_layers[2].layer_id, rcpt_c.layer_id);
    assert_eq!(issue.consumed_layers[2].quantity, 30);
    assert_eq!(issue.consumed_layers[2].extended_cost_minor, 36_000);

    // All layers fully consumed
    for (layer_id, label) in [
        (rcpt_a.layer_id, "A"),
        (rcpt_b.layer_id, "B"),
        (rcpt_c.layer_id, "C"),
    ] {
        let remaining: i64 =
            sqlx::query_scalar("SELECT quantity_remaining FROM inventory_layers WHERE id = $1")
                .bind(layer_id)
                .fetch_one(&pool)
                .await
                .expect("layer remaining");
        assert_eq!(remaining, 0, "Layer {} must be fully consumed", label);
    }

    cleanup_tenant(&pool, &tenant_id).await;
}

// ============================================================================
// Test 3: Partial consumption — precise layer split
// ============================================================================

/// Receives one large layer, issues half, verifies remaining is exact.
///
/// Layer A: 100 units @ $15.00
/// Issue 37 units → cost = 37 × 1500 = 55500 minor = $555.00
/// Remaining = 63 units
#[tokio::test]
#[serial]
async fn inventory_fifo_partial_consumption_exact() {
    let pool = get_inventory_pool().await;
    let tenant_id = format!("e2e-{}", Uuid::new_v4());

    let item = ItemRepo::create(&pool, &item_req(&tenant_id, "E2E-FIFO-PARTIAL-001"))
        .await
        .expect("create item");
    let warehouse_id = Uuid::new_v4();

    let (rcpt, _) = process_receipt(
        &pool,
        &receipt_req(&tenant_id, item.id, warehouse_id, 100, 1500),
        None,
    )
    .await
    .expect("receipt");

    let (issue, _) = process_issue(
        &pool,
        &issue_req(&tenant_id, item.id, warehouse_id, 37),
        None,
    )
    .await
    .expect("issue must succeed");

    assert_eq!(issue.quantity, 37);
    // 37 × 1500 = 55500
    assert_eq!(
        issue.total_cost_minor, 55_500,
        "cost = 37 × $15.00 = $555.00"
    );
    assert_eq!(issue.consumed_layers.len(), 1, "single layer");
    assert_eq!(issue.consumed_layers[0].layer_id, rcpt.layer_id);
    assert_eq!(issue.consumed_layers[0].quantity, 37);
    assert_eq!(issue.consumed_layers[0].extended_cost_minor, 55_500);

    // Remaining in layer
    let remaining: i64 =
        sqlx::query_scalar("SELECT quantity_remaining FROM inventory_layers WHERE id = $1")
            .bind(rcpt.layer_id)
            .fetch_one(&pool)
            .await
            .expect("layer remaining");
    assert_eq!(remaining, 63, "63 units remain (100 - 37)");

    // On-hand updated
    let on_hand: i64 = sqlx::query_scalar(
        "SELECT quantity_on_hand FROM item_on_hand WHERE tenant_id = $1 AND item_id = $2 AND warehouse_id = $3",
    )
    .bind(&tenant_id)
    .bind(item.id)
    .bind(warehouse_id)
    .fetch_one(&pool)
    .await
    .expect("on-hand");
    assert_eq!(on_hand, 63, "on-hand = 63 after issuing 37");

    cleanup_tenant(&pool, &tenant_id).await;
}

// ============================================================================
// Test 4: Consecutive issues drain layers deterministically
// ============================================================================

/// Issue multiple times sequentially from the same FIFO pool.
/// Verifies layer drain order and cost accumulation are deterministic.
///
/// Layer A: 20 units @ $10.00
/// Layer B: 20 units @ $30.00
///
/// Issue 1: 15 units → all from A (15 × 1000 = 15000)
/// Issue 2: 15 units → 5 from A + 10 from B (5×1000 + 10×3000 = 5000 + 30000 = 35000)
/// Issue 3: 10 units → remaining 10 from B (10 × 3000 = 30000)
#[tokio::test]
#[serial]
async fn inventory_fifo_consecutive_issues_deterministic() {
    let pool = get_inventory_pool().await;
    let tenant_id = format!("e2e-{}", Uuid::new_v4());

    let item = ItemRepo::create(&pool, &item_req(&tenant_id, "E2E-FIFO-SEQ-001"))
        .await
        .expect("create item");
    let warehouse_id = Uuid::new_v4();

    let (rcpt_a, _) = process_receipt(
        &pool,
        &receipt_req(&tenant_id, item.id, warehouse_id, 20, 1000),
        None,
    )
    .await
    .expect("Layer A");
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    let (rcpt_b, _) = process_receipt(
        &pool,
        &receipt_req(&tenant_id, item.id, warehouse_id, 20, 3000),
        None,
    )
    .await
    .expect("Layer B");

    // Issue 1: 15 units — all from Layer A
    let (issue1, _) = process_issue(
        &pool,
        &issue_req(&tenant_id, item.id, warehouse_id, 15),
        None,
    )
    .await
    .expect("issue 1");
    assert_eq!(issue1.total_cost_minor, 15_000, "issue1: 15 × $10 = $150");
    assert_eq!(issue1.consumed_layers[0].layer_id, rcpt_a.layer_id);
    assert_eq!(issue1.consumed_layers.len(), 1);

    // Issue 2: 15 units — 5 from Layer A remainder + 10 from Layer B
    let (issue2, _) = process_issue(
        &pool,
        &issue_req(&tenant_id, item.id, warehouse_id, 15),
        None,
    )
    .await
    .expect("issue 2");
    assert_eq!(
        issue2.total_cost_minor, 35_000,
        "issue2: 5×$10 + 10×$30 = $350"
    );
    assert_eq!(issue2.consumed_layers.len(), 2);
    assert_eq!(
        issue2.consumed_layers[0].layer_id, rcpt_a.layer_id,
        "A first"
    );
    assert_eq!(issue2.consumed_layers[0].quantity, 5);
    assert_eq!(
        issue2.consumed_layers[1].layer_id, rcpt_b.layer_id,
        "B second"
    );
    assert_eq!(issue2.consumed_layers[1].quantity, 10);

    // Issue 3: 10 units — all from Layer B remainder
    let (issue3, _) = process_issue(
        &pool,
        &issue_req(&tenant_id, item.id, warehouse_id, 10),
        None,
    )
    .await
    .expect("issue 3");
    assert_eq!(issue3.total_cost_minor, 30_000, "issue3: 10 × $30 = $300");
    assert_eq!(issue3.consumed_layers.len(), 1);
    assert_eq!(issue3.consumed_layers[0].layer_id, rcpt_b.layer_id);
    assert_eq!(issue3.consumed_layers[0].quantity, 10);

    // Both layers fully drained
    let a_rem: i64 =
        sqlx::query_scalar("SELECT quantity_remaining FROM inventory_layers WHERE id = $1")
            .bind(rcpt_a.layer_id)
            .fetch_one(&pool)
            .await
            .expect("layer A");
    assert_eq!(a_rem, 0, "Layer A fully drained");

    let b_rem: i64 =
        sqlx::query_scalar("SELECT quantity_remaining FROM inventory_layers WHERE id = $1")
            .bind(rcpt_b.layer_id)
            .fetch_one(&pool)
            .await
            .expect("layer B");
    assert_eq!(b_rem, 0, "Layer B fully drained");

    cleanup_tenant(&pool, &tenant_id).await;
}
