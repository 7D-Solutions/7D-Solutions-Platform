//! Integration tests for cycle count task creation (bd-2w8x).
//!
//! Tests run against a real PostgreSQL database.
//! Set DATABASE_URL to the inventory database connection string.
//!
//! Coverage:
//! 1. Full scope: task created, lines auto-populated from on-hand projection
//! 2. Partial scope: task created with explicit item list, expected_qty snapshotted
//! 3. Partial with unknown item: line created with expected_qty = 0
//! 4. Validation: partial scope with empty item_ids → error
//! 5. Validation: empty tenant_id → error
//! 6. Location guard: missing location → error
//! 7. Location guard: wrong tenant → error
//! 8. Full scope on empty location: task created, zero lines

use inventory_rs::domain::{
    cycle_count::task_service::{create_cycle_count_task, CreateTaskRequest, TaskError, TaskScope},
    items::{CreateItemRequest, ItemRepo, TrackingMode},
    locations::{CreateLocationRequest, LocationRepo},
    receipt_service::{process_receipt, ReceiptRequest},
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
        std::env::var("DATABASE_URL").unwrap_or_else(|_| "postgres://inventory_user:inventory_pass@localhost:5442/inventory_db?sslmode=disable".to_string());
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
    format!("cc-tenant-{}", Uuid::new_v4())
}

fn make_item_req(tenant_id: &str, sku: &str) -> CreateItemRequest {
    CreateItemRequest {
        tenant_id: tenant_id.to_string(),
        sku: sku.to_string(),
        name: format!("Test Item {}", sku),
        description: None,
        inventory_account_ref: "1200".to_string(),
        cogs_account_ref: "5000".to_string(),
        variance_account_ref: "5010".to_string(),
        uom: None,
        tracking_mode: TrackingMode::None,
        make_buy: None,
    }
}

fn make_location_req(tenant_id: &str, warehouse_id: Uuid, code: &str) -> CreateLocationRequest {
    CreateLocationRequest {
        tenant_id: tenant_id.to_string(),
        warehouse_id,
        code: code.to_string(),
        name: format!("Location {}", code),
        description: None,
    }
}

fn receipt_req_at(
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
    location_id: Uuid,
    qty: i64,
    key: &str,
) -> ReceiptRequest {
    ReceiptRequest {
        tenant_id: tenant_id.to_string(),
        item_id,
        warehouse_id,
        location_id: Some(location_id),
        quantity: qty,
        unit_cost_minor: 1000,
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

// ============================================================================
// Tests
// ============================================================================

/// 1. Full scope: task created; lines auto-populated from on-hand at the location.
#[tokio::test]
#[serial]
async fn test_full_scope_populates_lines_from_on_hand() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let warehouse_id = Uuid::new_v4();

    let item = ItemRepo::create(&pool, &make_item_req(&tenant, "CC-FULL-01"))
        .await
        .expect("create item");

    let loc = LocationRepo::create(&pool, &make_location_req(&tenant, warehouse_id, "A-01"))
        .await
        .expect("create location");

    // Put stock at the location
    process_receipt(
        &pool,
        &receipt_req_at(&tenant, item.id, warehouse_id, loc.id, 50, "cc-rcpt-1"),
        None,
    )
    .await
    .expect("receipt");

    let req = CreateTaskRequest {
        tenant_id: tenant.clone(),
        warehouse_id,
        location_id: loc.id,
        scope: TaskScope::Full,
        item_ids: vec![],
    };
    let result = create_cycle_count_task(&pool, &req)
        .await
        .expect("create task");

    assert_eq!(result.status, "open");
    assert_eq!(result.scope, TaskScope::Full);
    assert_eq!(result.line_count, 1);
    assert_eq!(result.lines[0].item_id, item.id);
    assert_eq!(result.lines[0].expected_qty, 50);
}

/// 2. Partial scope: caller provides item list, expected_qty is snapshotted.
#[tokio::test]
#[serial]
async fn test_partial_scope_uses_caller_item_list() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let warehouse_id = Uuid::new_v4();

    let item_a = ItemRepo::create(&pool, &make_item_req(&tenant, "CC-PART-A"))
        .await
        .expect("create item A");
    let item_b = ItemRepo::create(&pool, &make_item_req(&tenant, "CC-PART-B"))
        .await
        .expect("create item B");
    let _ = item_b; // used only to confirm it won't appear in the partial count

    let loc = LocationRepo::create(&pool, &make_location_req(&tenant, warehouse_id, "B-02"))
        .await
        .expect("create location");

    // Receipt for both items
    process_receipt(
        &pool,
        &receipt_req_at(&tenant, item_a.id, warehouse_id, loc.id, 30, "cc-p-rcpt-a"),
        None,
    )
    .await
    .expect("receipt A");
    process_receipt(
        &pool,
        &receipt_req_at(&tenant, item_b.id, warehouse_id, loc.id, 20, "cc-p-rcpt-b"),
        None,
    )
    .await
    .expect("receipt B");

    // Only request a partial count for item_a
    let req = CreateTaskRequest {
        tenant_id: tenant.clone(),
        warehouse_id,
        location_id: loc.id,
        scope: TaskScope::Partial,
        item_ids: vec![item_a.id],
    };
    let result = create_cycle_count_task(&pool, &req)
        .await
        .expect("create task");

    assert_eq!(result.line_count, 1);
    assert_eq!(result.lines[0].item_id, item_a.id);
    assert_eq!(result.lines[0].expected_qty, 30);
}

/// 3. Partial with unknown item: line created with expected_qty = 0.
#[tokio::test]
#[serial]
async fn test_partial_unknown_item_gets_zero_expected() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let warehouse_id = Uuid::new_v4();

    let known_item = ItemRepo::create(&pool, &make_item_req(&tenant, "CC-UNK-KNOWN"))
        .await
        .expect("create known item");

    // Create a second item that will be used in the count but has no on-hand
    let unknown_item = ItemRepo::create(&pool, &make_item_req(&tenant, "CC-UNK-ZERO"))
        .await
        .expect("create zero-stock item");

    let loc = LocationRepo::create(&pool, &make_location_req(&tenant, warehouse_id, "C-03"))
        .await
        .expect("create location");

    // Only the known item has stock
    process_receipt(
        &pool,
        &receipt_req_at(
            &tenant,
            known_item.id,
            warehouse_id,
            loc.id,
            10,
            "cc-unk-rcpt",
        ),
        None,
    )
    .await
    .expect("receipt");

    let req = CreateTaskRequest {
        tenant_id: tenant.clone(),
        warehouse_id,
        location_id: loc.id,
        scope: TaskScope::Partial,
        item_ids: vec![unknown_item.id],
    };
    let result = create_cycle_count_task(&pool, &req)
        .await
        .expect("create task");

    assert_eq!(result.line_count, 1);
    assert_eq!(result.lines[0].item_id, unknown_item.id);
    assert_eq!(result.lines[0].expected_qty, 0);
}

/// 4. Partial scope with empty item_ids → validation error.
#[tokio::test]
#[serial]
async fn test_partial_empty_item_ids_rejected() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let warehouse_id = Uuid::new_v4();

    let loc = LocationRepo::create(&pool, &make_location_req(&tenant, warehouse_id, "D-04"))
        .await
        .expect("create location");

    let req = CreateTaskRequest {
        tenant_id: tenant.clone(),
        warehouse_id,
        location_id: loc.id,
        scope: TaskScope::Partial,
        item_ids: vec![],
    };
    let err = create_cycle_count_task(&pool, &req)
        .await
        .expect_err("should fail");

    assert!(matches!(err, TaskError::EmptyPartialItemList));
}

/// 5. Empty tenant_id → validation error.
#[tokio::test]
#[serial]
async fn test_empty_tenant_rejected() {
    let pool = setup_db().await;

    let req = CreateTaskRequest {
        tenant_id: "".to_string(),
        warehouse_id: Uuid::new_v4(),
        location_id: Uuid::new_v4(),
        scope: TaskScope::Full,
        item_ids: vec![],
    };
    let err = create_cycle_count_task(&pool, &req)
        .await
        .expect_err("should fail");

    assert!(matches!(err, TaskError::MissingTenant));
}

/// 6. Missing location → guard error.
#[tokio::test]
#[serial]
async fn test_missing_location_rejected() {
    let pool = setup_db().await;
    let tenant = unique_tenant();

    let req = CreateTaskRequest {
        tenant_id: tenant.clone(),
        warehouse_id: Uuid::new_v4(),
        location_id: Uuid::new_v4(), // does not exist
        scope: TaskScope::Full,
        item_ids: vec![],
    };
    let err = create_cycle_count_task(&pool, &req)
        .await
        .expect_err("should fail");

    assert!(matches!(err, TaskError::LocationNotFound));
}

/// 7. Location belongs to a different tenant → guard error.
#[tokio::test]
#[serial]
async fn test_wrong_tenant_location_rejected() {
    let pool = setup_db().await;
    let tenant_a = unique_tenant();
    let tenant_b = unique_tenant();
    let warehouse_id = Uuid::new_v4();

    // Create location under tenant_a
    let loc = LocationRepo::create(&pool, &make_location_req(&tenant_a, warehouse_id, "E-05"))
        .await
        .expect("create location for tenant_a");

    // Attempt to create cycle count task as tenant_b
    let req = CreateTaskRequest {
        tenant_id: tenant_b.clone(),
        warehouse_id,
        location_id: loc.id,
        scope: TaskScope::Full,
        item_ids: vec![],
    };
    let err = create_cycle_count_task(&pool, &req)
        .await
        .expect_err("should fail");

    assert!(matches!(err, TaskError::LocationNotFound));
}

/// 8. Full scope on empty location: task is created with zero lines.
#[tokio::test]
#[serial]
async fn test_full_scope_empty_location_creates_zero_lines() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let warehouse_id = Uuid::new_v4();

    let loc = LocationRepo::create(&pool, &make_location_req(&tenant, warehouse_id, "F-06"))
        .await
        .expect("create location");

    let req = CreateTaskRequest {
        tenant_id: tenant.clone(),
        warehouse_id,
        location_id: loc.id,
        scope: TaskScope::Full,
        item_ids: vec![],
    };
    let result = create_cycle_count_task(&pool, &req)
        .await
        .expect("create task");

    assert_eq!(result.status, "open");
    assert_eq!(result.line_count, 0);
    assert!(result.lines.is_empty());
}
