//! Integration tests for valuation snapshot builder (bd-2k0i).
//!
//! Tests run against a real PostgreSQL database.
//! Set DATABASE_URL to the inventory database connection string.
//!
//! Coverage:
//! 1. Empty warehouse → snapshot created with 0 lines, total_value_minor = 0
//! 2. Single item → qty and weighted-avg cost computed correctly
//! 3. Multiple items → totals aggregated correctly
//! 4. Partial consumption before as_of → remaining qty reflects consumed layers
//! 5. Idempotent replay → same result returned on duplicate key
//! 6. Conflicting idempotency key → ConflictingIdempotencyKey error
//! 7. Missing tenant_id → MissingTenant error
//! 8. Outbox event emitted after snapshot creation

use chrono::Utc;
use inventory_rs::domain::{
    issue_service::{process_issue, IssueRequest},
    items::{CreateItemRequest, ItemRepo, TrackingMode},
    receipt_service::{process_receipt, ReceiptRequest},
    valuation::snapshot_service::{
        create_valuation_snapshot, CreateSnapshotRequest, SnapshotError,
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
    format!("vs-tenant-{}", Uuid::new_v4())
}

fn make_item_req(tenant_id: &str, sku: &str) -> CreateItemRequest {
    CreateItemRequest {
        tenant_id: tenant_id.to_string(),
        sku: sku.to_string(),
        name: format!("Valuation Test Item {}", sku),
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
        source_type: "purchase".to_string(),
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

fn snapshot_req(tenant_id: &str, warehouse_id: Uuid, key: &str) -> CreateSnapshotRequest {
    CreateSnapshotRequest {
        tenant_id: tenant_id.to_string(),
        warehouse_id,
        location_id: None,
        as_of: Utc::now(),
        idempotency_key: key.to_string(),
        currency: "usd".to_string(),
        correlation_id: None,
        causation_id: None,
    }
}

// ============================================================================
// Tests
// ============================================================================

/// Empty warehouse → 0 lines, total_value_minor = 0.
#[tokio::test]
#[serial]
async fn test_snapshot_empty_warehouse() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let warehouse_id = Uuid::new_v4();

    let req = snapshot_req(&tenant, warehouse_id, "snap-empty-001");
    let (result, is_replay) = create_valuation_snapshot(&pool, &req)
        .await
        .expect("create snapshot");

    assert!(!is_replay);
    assert_eq!(result.line_count, 0);
    assert_eq!(result.total_value_minor, 0);
    assert!(result.lines.is_empty());
    assert_eq!(result.warehouse_id, warehouse_id);
    assert_eq!(result.tenant_id, tenant);
}

/// Single item → correct qty and value computed.
#[tokio::test]
#[serial]
async fn test_snapshot_single_item_correct_value() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let warehouse_id = Uuid::new_v4();

    let item = ItemRepo::create(&pool, &make_item_req(&tenant, "VAL-A01"))
        .await
        .expect("create item");

    // Receive 20 units @ $10.00 each
    process_receipt(
        &pool,
        &receipt_req(&tenant, item.id, warehouse_id, 20, 1000, "r1"),
        None,
    )
    .await
    .expect("receipt");

    let req = snapshot_req(&tenant, warehouse_id, "snap-single-001");
    let (result, is_replay) = create_valuation_snapshot(&pool, &req)
        .await
        .expect("snapshot");

    assert!(!is_replay);
    assert_eq!(result.line_count, 1);
    let line = &result.lines[0];
    assert_eq!(line.item_id, item.id);
    assert_eq!(line.quantity_on_hand, 20);
    assert_eq!(line.unit_cost_minor, 1000);
    assert_eq!(line.total_value_minor, 20_000);
    assert_eq!(result.total_value_minor, 20_000);
}

/// Multiple items → totals aggregated.
#[tokio::test]
#[serial]
async fn test_snapshot_multiple_items_totals() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let warehouse_id = Uuid::new_v4();

    let item_a = ItemRepo::create(&pool, &make_item_req(&tenant, "VAL-M01"))
        .await
        .expect("create item a");
    let item_b = ItemRepo::create(&pool, &make_item_req(&tenant, "VAL-M02"))
        .await
        .expect("create item b");

    // Item A: 10 units @ $5.00 = $50
    process_receipt(
        &pool,
        &receipt_req(&tenant, item_a.id, warehouse_id, 10, 500, "r-ma1"),
        None,
    )
    .await
    .expect("receipt a");

    // Item B: 4 units @ $25.00 = $100
    process_receipt(
        &pool,
        &receipt_req(&tenant, item_b.id, warehouse_id, 4, 2500, "r-mb1"),
        None,
    )
    .await
    .expect("receipt b");

    let req = snapshot_req(&tenant, warehouse_id, "snap-multi-001");
    let (result, _) = create_valuation_snapshot(&pool, &req)
        .await
        .expect("snapshot");

    assert_eq!(result.line_count, 2);
    assert_eq!(result.total_value_minor, 15_000); // $50 + $100

    let totals: i64 = result.lines.iter().map(|l| l.total_value_minor).sum();
    assert_eq!(totals, result.total_value_minor);
}

/// Issue before as_of → remaining qty reflects consumption.
#[tokio::test]
#[serial]
async fn test_snapshot_reflects_consumption() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let warehouse_id = Uuid::new_v4();

    let item = ItemRepo::create(&pool, &make_item_req(&tenant, "VAL-C01"))
        .await
        .expect("create item");

    // Receive 50 units @ $2.00 = $100 total
    process_receipt(
        &pool,
        &receipt_req(&tenant, item.id, warehouse_id, 50, 200, "r-c1"),
        None,
    )
    .await
    .expect("receipt");

    // Issue 30 units (consume 30 layers)
    process_issue(
        &pool,
        &issue_req(&tenant, item.id, warehouse_id, 30, "i-c1"),
        None,
    )
    .await
    .expect("issue");

    // as_of = now → should see 20 units remaining
    let req = snapshot_req(&tenant, warehouse_id, "snap-consume-001");
    let (result, _) = create_valuation_snapshot(&pool, &req)
        .await
        .expect("snapshot");

    assert_eq!(result.line_count, 1);
    let line = &result.lines[0];
    assert_eq!(line.quantity_on_hand, 20);
    assert_eq!(line.unit_cost_minor, 200);
    assert_eq!(line.total_value_minor, 4_000);
    assert_eq!(result.total_value_minor, 4_000);
}

/// Idempotent replay → same result returned on duplicate key.
#[tokio::test]
#[serial]
async fn test_snapshot_idempotent_replay() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let warehouse_id = Uuid::new_v4();

    let req = snapshot_req(&tenant, warehouse_id, "snap-idem-001");

    let (first, is_replay_1) = create_valuation_snapshot(&pool, &req)
        .await
        .expect("first create");
    assert!(!is_replay_1);

    let (second, is_replay_2) = create_valuation_snapshot(&pool, &req)
        .await
        .expect("second create");
    assert!(is_replay_2);

    assert_eq!(first.snapshot_id, second.snapshot_id);
    assert_eq!(first.total_value_minor, second.total_value_minor);
}

/// Conflicting idempotency key (same key, different body) → error.
#[tokio::test]
#[serial]
async fn test_snapshot_conflicting_idempotency_key() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let warehouse_id_1 = Uuid::new_v4();
    let warehouse_id_2 = Uuid::new_v4();

    let req1 = snapshot_req(&tenant, warehouse_id_1, "snap-conflict-001");
    create_valuation_snapshot(&pool, &req1)
        .await
        .expect("first create");

    // Same key, different warehouse_id → body hash differs
    let req2 = CreateSnapshotRequest {
        tenant_id: tenant.clone(),
        warehouse_id: warehouse_id_2,
        idempotency_key: "snap-conflict-001".to_string(),
        as_of: req1.as_of,
        ..snapshot_req(&tenant, warehouse_id_2, "snap-conflict-001")
    };
    let err = create_valuation_snapshot(&pool, &req2)
        .await
        .expect_err("should fail");
    assert!(matches!(err, SnapshotError::ConflictingIdempotencyKey));
}

/// Missing tenant_id → MissingTenant error.
#[tokio::test]
#[serial]
async fn test_snapshot_missing_tenant() {
    let pool = setup_db().await;
    let req = CreateSnapshotRequest {
        tenant_id: "".to_string(),
        warehouse_id: Uuid::new_v4(),
        location_id: None,
        as_of: Utc::now(),
        idempotency_key: "snap-notenant-001".to_string(),
        currency: "usd".to_string(),
        correlation_id: None,
        causation_id: None,
    };
    let err = create_valuation_snapshot(&pool, &req)
        .await
        .expect_err("should fail");
    assert!(matches!(err, SnapshotError::MissingTenant));
}

/// Outbox event emitted after snapshot creation.
#[tokio::test]
#[serial]
async fn test_snapshot_outbox_event_emitted() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let warehouse_id = Uuid::new_v4();

    let req = snapshot_req(&tenant, warehouse_id, "snap-outbox-001");
    let (result, _) = create_valuation_snapshot(&pool, &req)
        .await
        .expect("create snapshot");

    let row: Option<(String,)> =
        sqlx::query_as("SELECT event_type FROM inv_outbox WHERE aggregate_id = $1")
            .bind(result.snapshot_id.to_string())
            .fetch_optional(&pool)
            .await
            .expect("query outbox");

    let (event_type,) = row.expect("outbox event should be present");
    assert_eq!(event_type, "inventory.valuation_snapshot_created");
}

/// Tenant isolation: snapshots are scoped to tenant.
#[tokio::test]
#[serial]
async fn test_snapshot_tenant_isolation() {
    let pool = setup_db().await;
    let tenant_a = unique_tenant();
    let tenant_b = unique_tenant();
    let warehouse_id = Uuid::new_v4();

    let item_a = ItemRepo::create(&pool, &make_item_req(&tenant_a, "VAL-TA01"))
        .await
        .expect("create item a");

    // Tenant A receives stock
    process_receipt(
        &pool,
        &receipt_req(&tenant_a, item_a.id, warehouse_id, 100, 500, "r-ta1"),
        None,
    )
    .await
    .expect("receipt a");

    // Tenant B snapshot — should see no stock from tenant A
    let req_b = snapshot_req(&tenant_b, warehouse_id, "snap-tb-001");
    let (result_b, _) = create_valuation_snapshot(&pool, &req_b)
        .await
        .expect("snapshot b");

    assert_eq!(result_b.line_count, 0);
    assert_eq!(result_b.total_value_minor, 0);
}
