//! Integration tests for multi-method valuation (bd-3n7jh).
//!
//! Tests run against a real PostgreSQL database on port 5442.
//! No mocks, no stubs — all operations hit the real inventory service.
//!
//! Coverage:
//! 1. LIFO valuation E2E
//! 2. WAC valuation E2E
//! 3. Standard cost E2E with variance
//! 4. Method comparison (same data, all 4 methods produce correct different values)
//! 5. Tenant isolation
//! 6. Idempotency
//! 7. Outbox event with correct method, totals, and tenant_id

use chrono::Utc;
use inventory_rs::domain::{
    items::{CreateItemRequest, ItemRepo, TrackingMode},
    receipt_service::{process_receipt, ReceiptRequest},
    issue_service::{process_issue, IssueRequest},
    valuation::{
        methods::ValuationMethod,
        run_service::{
            execute_valuation_run, set_item_valuation_method, RunError, ValuationRunRequest,
        },
    },
};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

// ============================================================================
// DB helpers
// ============================================================================

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| "postgres://inventory_user:inventory_pass@localhost:5442/inventory_db".to_string());
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to inventory test DB");
    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run inventory migrations");
    pool
}

fn unique_tenant() -> String {
    format!("vm-tenant-{}", Uuid::new_v4())
}

fn make_item_req(tenant_id: &str, sku: &str) -> CreateItemRequest {
    CreateItemRequest {
        tenant_id: tenant_id.to_string(),
        sku: sku.to_string(),
        name: format!("Valuation Method Test {}", sku),
        description: None,
        inventory_account_ref: "1200".to_string(),
        cogs_account_ref: "5000".to_string(),
        variance_account_ref: "5010".to_string(),
        uom: None,
        tracking_mode: TrackingMode::None,
    }
}

fn receipt_req(
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
    qty: i64,
    unit_cost: i64,
    key: &str,
) -> ReceiptRequest {
    ReceiptRequest {
        tenant_id: tenant_id.to_string(),
        item_id,
        warehouse_id,
        location_id: None,
        quantity: qty,
        unit_cost_minor: unit_cost,
        currency: "usd".to_string(),
        purchase_order_id: None,
        idempotency_key: key.to_string(),
        correlation_id: None,
        causation_id: None,
        lot_code: None,
        serial_codes: None,
        uom_id: None,
    }
}

fn issue_req(
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
    qty: i64,
    key: &str,
) -> IssueRequest {
    IssueRequest {
        tenant_id: tenant_id.to_string(),
        item_id,
        warehouse_id,
        location_id: None,
        quantity: qty,
        currency: "usd".to_string(),
        source_module: "test".to_string(),
        source_type: "test_order".to_string(),
        source_id: Uuid::new_v4().to_string(),
        source_line_id: None,
        idempotency_key: key.to_string(),
        correlation_id: None,
        causation_id: None,
        lot_code: None,
        serial_codes: None,
        uom_id: None,
    }
}

fn run_req(
    tenant_id: &str,
    warehouse_id: Uuid,
    method: ValuationMethod,
    key: &str,
) -> ValuationRunRequest {
    ValuationRunRequest {
        tenant_id: tenant_id.to_string(),
        warehouse_id,
        method,
        as_of: Utc::now(),
        idempotency_key: key.to_string(),
        currency: "usd".to_string(),
        correlation_id: None,
        causation_id: None,
    }
}

/// Seed inventory: create item, receive stock at different costs, issue some.
///
/// Returns (item_id, warehouse_id).
///
/// Receipts:
///   - 10 @ $5.00 (oldest)
///   - 20 @ $8.00 (mid)
///   - 15 @ $10.00 (newest)
/// Issue: 25 units (FIFO consumes oldest layers)
/// On hand: 20 units
async fn seed_multi_layer(pool: &sqlx::PgPool, tenant: &str) -> (Uuid, Uuid) {
    let warehouse_id = Uuid::new_v4();
    let suffix = &Uuid::new_v4().to_string()[..8];

    let item = ItemRepo::create(pool, &make_item_req(tenant, &format!("VM-{suffix}")))
        .await
        .expect("create item");

    // Receipt 1: 10 @ $5.00
    process_receipt(
        pool,
        &receipt_req(tenant, item.id, warehouse_id, 10, 500, &format!("r1-{suffix}")),
        None,
    )
    .await
    .expect("receipt 1");

    // Receipt 2: 20 @ $8.00
    process_receipt(
        pool,
        &receipt_req(tenant, item.id, warehouse_id, 20, 800, &format!("r2-{suffix}")),
        None,
    )
    .await
    .expect("receipt 2");

    // Receipt 3: 15 @ $10.00
    process_receipt(
        pool,
        &receipt_req(tenant, item.id, warehouse_id, 15, 1000, &format!("r3-{suffix}")),
        None,
    )
    .await
    .expect("receipt 3");

    // Issue 25 units (FIFO: consumes 10@$5 + 15@$8)
    process_issue(
        pool,
        &issue_req(tenant, item.id, warehouse_id, 25, &format!("i1-{suffix}")),
        None,
    )
    .await
    .expect("issue");

    (item.id, warehouse_id)
}

// ============================================================================
// Test 1: LIFO valuation E2E
// ============================================================================

/// LIFO: newest purchases consumed first, remaining valued at oldest costs.
///
/// Receipts: 10@$5 + 20@$8 + 15@$10 = 45 units
/// Issue: 25 units consumed
/// On hand: 20 units
///
/// Under LIFO, conceptual consumption: 15@$10 + 10@$8 = 25 consumed from newest
/// Remaining: 10@$5 + 10@$8 = $50 + $80 = $130
#[tokio::test]
#[serial]
async fn test_lifo_valuation_e2e() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let (item_id, warehouse_id) = seed_multi_layer(&pool, &tenant).await;

    let req = run_req(&tenant, warehouse_id, ValuationMethod::Lifo, "lifo-e2e-001");
    let (result, is_replay) = execute_valuation_run(&pool, &req)
        .await
        .expect("lifo run");

    assert!(!is_replay);
    assert_eq!(result.method, "lifo");
    assert_eq!(result.line_count, 1);

    let line = &result.lines[0];
    assert_eq!(line.item_id, item_id);
    assert_eq!(line.quantity_on_hand, 20);

    // LIFO: remaining = oldest layers
    // 10@$5 + 10@$8 = 5000 + 8000 = 13000
    assert_eq!(line.total_value_minor, 13000);
    assert_eq!(result.total_value_minor, 13000);
}

// ============================================================================
// Test 2: WAC valuation E2E
// ============================================================================

/// WAC: weighted average of all purchase costs applied uniformly.
///
/// WAC = (10*500 + 20*800 + 15*1000) / (10+20+15) = 36000/45 = 800
/// Value = 20 * 800 = 16000
#[tokio::test]
#[serial]
async fn test_wac_valuation_e2e() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let (_item_id, warehouse_id) = seed_multi_layer(&pool, &tenant).await;

    let req = run_req(&tenant, warehouse_id, ValuationMethod::Wac, "wac-e2e-001");
    let (result, is_replay) = execute_valuation_run(&pool, &req)
        .await
        .expect("wac run");

    assert!(!is_replay);
    assert_eq!(result.method, "wac");
    assert_eq!(result.line_count, 1);

    let line = &result.lines[0];
    assert_eq!(line.quantity_on_hand, 20);
    assert_eq!(line.unit_cost_minor, 800); // WAC = 36000/45 = 800
    assert_eq!(line.total_value_minor, 16000); // 20 * 800
    assert_eq!(result.total_value_minor, 16000);
}

// ============================================================================
// Test 3: Standard cost E2E with variance
// ============================================================================

/// Standard cost at $7.00 per unit.
/// Value = 20 * 700 = 14000
/// Actual FIFO value = 5*800 + 15*1000 = 4000 + 15000 = 19000
/// Wait — FIFO remaining: 10 of receipt 1 consumed, 15 of receipt 2 consumed,
/// so 5@$8 remain from r2 and all 15@$10 from r3.
/// Actually: r1=10@$5 fully consumed, r2=20@$8 with 15 consumed (5 remain),
/// r3=15@$10 fully remaining (0 consumed from r3).
/// Actual FIFO value: 5*800 + 15*1000 = 4000 + 15000 = 19000
/// Variance: 19000 - 14000 = 5000 (unfavorable, actual > standard)
#[tokio::test]
#[serial]
async fn test_standard_cost_e2e() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let (item_id, warehouse_id) = seed_multi_layer(&pool, &tenant).await;

    // Configure standard cost for this item
    set_item_valuation_method(
        &pool,
        &tenant,
        item_id,
        ValuationMethod::StandardCost,
        Some(700),
    )
    .await
    .expect("set standard cost");

    let req = run_req(
        &tenant,
        warehouse_id,
        ValuationMethod::StandardCost,
        "std-e2e-001",
    );
    let (result, is_replay) = execute_valuation_run(&pool, &req)
        .await
        .expect("standard cost run");

    assert!(!is_replay);
    assert_eq!(result.method, "standard_cost");
    assert_eq!(result.line_count, 1);

    let line = &result.lines[0];
    assert_eq!(line.quantity_on_hand, 20);
    assert_eq!(line.unit_cost_minor, 700);
    assert_eq!(line.total_value_minor, 14000); // 20 * 700

    // Variance: actual FIFO (5*800 + 15*1000 = 19000) - standard (14000) = 5000
    assert_eq!(line.variance_minor, 5000);
}

// ============================================================================
// Test 4: Method comparison
// ============================================================================

/// Same inventory data, all 4 methods should produce different valuations.
#[tokio::test]
#[serial]
async fn test_method_comparison() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let (item_id, warehouse_id) = seed_multi_layer(&pool, &tenant).await;

    // Configure standard cost for the standard_cost run
    set_item_valuation_method(
        &pool,
        &tenant,
        item_id,
        ValuationMethod::StandardCost,
        Some(700),
    )
    .await
    .expect("set standard cost");

    let fifo_req = run_req(&tenant, warehouse_id, ValuationMethod::Fifo, "cmp-fifo");
    let lifo_req = run_req(&tenant, warehouse_id, ValuationMethod::Lifo, "cmp-lifo");
    let wac_req = run_req(&tenant, warehouse_id, ValuationMethod::Wac, "cmp-wac");
    let std_req = run_req(
        &tenant,
        warehouse_id,
        ValuationMethod::StandardCost,
        "cmp-std",
    );

    let (fifo, _) = execute_valuation_run(&pool, &fifo_req)
        .await
        .expect("fifo run");
    let (lifo, _) = execute_valuation_run(&pool, &lifo_req)
        .await
        .expect("lifo run");
    let (wac, _) = execute_valuation_run(&pool, &wac_req)
        .await
        .expect("wac run");
    let (std_cost, _) = execute_valuation_run(&pool, &std_req)
        .await
        .expect("standard cost run");

    // All agree on quantity
    assert_eq!(fifo.lines[0].quantity_on_hand, 20);
    assert_eq!(lifo.lines[0].quantity_on_hand, 20);
    assert_eq!(wac.lines[0].quantity_on_hand, 20);
    assert_eq!(std_cost.lines[0].quantity_on_hand, 20);

    // FIFO: remaining = newest layers: 5@$8 + 15@$10 = 19000
    assert_eq!(fifo.total_value_minor, 19000);
    // LIFO: remaining = oldest layers: 10@$5 + 10@$8 = 13000
    assert_eq!(lifo.total_value_minor, 13000);
    // WAC: 20 * 800 = 16000
    assert_eq!(wac.total_value_minor, 16000);
    // Standard: 20 * 700 = 14000
    assert_eq!(std_cost.total_value_minor, 14000);

    // Verify all are different
    let values = [
        fifo.total_value_minor,
        lifo.total_value_minor,
        wac.total_value_minor,
        std_cost.total_value_minor,
    ];
    for i in 0..values.len() {
        for j in (i + 1)..values.len() {
            assert_ne!(
                values[i], values[j],
                "method {} and {} should produce different values",
                i, j
            );
        }
    }
}

// ============================================================================
// Test 5: Tenant isolation
// ============================================================================

/// Valuation data from tenant_A must not appear in tenant_B's run.
#[tokio::test]
#[serial]
async fn test_tenant_isolation() {
    let pool = setup_db().await;
    let tenant_a = unique_tenant();
    let tenant_b = unique_tenant();
    let warehouse_id = Uuid::new_v4();
    let suffix = &Uuid::new_v4().to_string()[..8];

    // Tenant A: create item and receive stock
    let item_a = ItemRepo::create(&pool, &make_item_req(&tenant_a, &format!("TI-A-{suffix}")))
        .await
        .expect("create item a");

    process_receipt(
        &pool,
        &receipt_req(
            &tenant_a,
            item_a.id,
            warehouse_id,
            100,
            500,
            &format!("r-ta-{suffix}"),
        ),
        None,
    )
    .await
    .expect("receipt a");

    // Tenant B: run valuation — should see zero stock from tenant A
    let req_b = run_req(&tenant_b, warehouse_id, ValuationMethod::Fifo, &format!("iso-b-{suffix}"));
    let (result_b, _) = execute_valuation_run(&pool, &req_b)
        .await
        .expect("run b");

    assert_eq!(result_b.line_count, 0);
    assert_eq!(result_b.total_value_minor, 0);
    assert!(result_b.lines.is_empty());

    // Tenant A: run valuation — should see their stock
    let req_a = run_req(
        &tenant_a,
        warehouse_id,
        ValuationMethod::Fifo,
        &format!("iso-a-{suffix}"),
    );
    let (result_a, _) = execute_valuation_run(&pool, &req_a)
        .await
        .expect("run a");

    assert_eq!(result_a.line_count, 1);
    assert_eq!(result_a.total_value_minor, 50000); // 100 * 500
}

// ============================================================================
// Test 6: Idempotency
// ============================================================================

/// Same idempotency_key + same body → replay with same result.
/// Same key + different body → ConflictingIdempotencyKey error.
#[tokio::test]
#[serial]
async fn test_idempotency() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let warehouse_id = Uuid::new_v4();

    let req = run_req(&tenant, warehouse_id, ValuationMethod::Lifo, "idem-001");

    // First call
    let (first, is_replay_1) = execute_valuation_run(&pool, &req)
        .await
        .expect("first run");
    assert!(!is_replay_1);

    // Second call with same key + same body → replay
    let (second, is_replay_2) = execute_valuation_run(&pool, &req)
        .await
        .expect("replay run");
    assert!(is_replay_2);
    assert_eq!(first.run_id, second.run_id);
    assert_eq!(first.total_value_minor, second.total_value_minor);

    // Check no duplicate run records in DB
    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM valuation_runs WHERE tenant_id = $1 AND id = $2",
    )
    .bind(&tenant)
    .bind(first.run_id)
    .fetch_one(&pool)
    .await
    .expect("count query");
    assert_eq!(count.0, 1, "no duplicate run records");

    // Different body with same key → conflict
    let different_wh = Uuid::new_v4();
    let conflict_req = run_req(&tenant, different_wh, ValuationMethod::Lifo, "idem-001");
    let err = execute_valuation_run(&pool, &conflict_req)
        .await
        .expect_err("should conflict");
    assert!(
        matches!(err, RunError::ConflictingIdempotencyKey),
        "expected ConflictingIdempotencyKey, got {:?}",
        err
    );
}

// ============================================================================
// Test 7: Outbox event
// ============================================================================

/// After valuation run, outbox must contain an event with correct method,
/// totals, and tenant_id.
#[tokio::test]
#[serial]
async fn test_outbox_event() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let (_item_id, warehouse_id) = seed_multi_layer(&pool, &tenant).await;

    let req = run_req(&tenant, warehouse_id, ValuationMethod::Wac, "outbox-001");
    let (result, _) = execute_valuation_run(&pool, &req)
        .await
        .expect("valuation run");

    // Query outbox for the event
    let row: Option<(String, String, String)> = sqlx::query_as(
        r#"
        SELECT event_type, tenant_id, payload::TEXT
        FROM inv_outbox
        WHERE tenant_id = $1
          AND aggregate_type = 'valuation_run'
          AND aggregate_id = $2
        "#,
    )
    .bind(&tenant)
    .bind(result.run_id.to_string())
    .fetch_optional(&pool)
    .await
    .expect("outbox query");

    let (event_type, outbox_tenant, payload_text) = row.expect("outbox event must exist");
    assert_eq!(event_type, "inventory.valuation_run_completed");
    assert_eq!(outbox_tenant, tenant);

    // Parse payload to verify method and totals
    let envelope: serde_json::Value =
        serde_json::from_str(&payload_text).expect("parse outbox payload");
    let payload_data = &envelope["payload"];
    assert_eq!(payload_data["method"], "wac");
    assert_eq!(payload_data["tenant_id"], tenant);
    assert_eq!(payload_data["total_value_minor"], 16000);
    assert_eq!(payload_data["line_count"], 1);
}
