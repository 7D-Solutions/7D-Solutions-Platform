//! Integration tests for the location model (bd-vzlc).
//!
//! Covers:
//! 1. Create location — CRUD happy path
//! 2. Duplicate code rejected
//! 3. Receipt with location_id → ledger has location_id; on_hand row keyed by location
//! 4. Receipt without location_id → null-location on_hand row (backward compat)
//! 5. Issue with location_id → deducts from location-specific on_hand
//! 6. Issue without location_id → deducts from null-location on_hand (backward compat)
//! 7. List locations by warehouse

use inventory_rs::domain::{
    issue_service::{process_issue, IssueRequest, IssueResult},
    items::{CreateItemRequest, ItemRepo, TrackingMode},
    locations::{CreateLocationRequest, LocationError, LocationRepo, UpdateLocationRequest},
    receipt_service::{process_receipt, ReceiptRequest, ReceiptResult},
};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

// ============================================================================
// Test DB helpers
// ============================================================================

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://inventory_user:inventory_pass@localhost:5442/inventory_db?sslmode=require"
            .to_string()
    });

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

fn item_req(tenant_id: &str, sku: &str) -> CreateItemRequest {
    CreateItemRequest {
        tenant_id: tenant_id.to_string(),
        sku: sku.to_string(),
        name: "Test Widget".to_string(),
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
    location_id: Option<Uuid>,
    idem_key: &str,
) -> ReceiptRequest {
    ReceiptRequest {
        tenant_id: tenant_id.to_string(),
        item_id,
        warehouse_id,
        location_id,
        quantity: 50,
        unit_cost_minor: 10_00,
        currency: "usd".to_string(),
        source_type: "purchase".to_string(),
        purchase_order_id: None,
        idempotency_key: idem_key.to_string(),
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
    location_id: Option<Uuid>,
    quantity: i64,
    idem_key: &str,
) -> IssueRequest {
    IssueRequest {
        tenant_id: tenant_id.to_string(),
        item_id,
        warehouse_id,
        location_id,
        quantity,
        currency: "usd".to_string(),
        source_module: "orders".to_string(),
        source_type: "sales_order".to_string(),
        source_id: "SO-001".to_string(),
        source_line_id: None,
        idempotency_key: idem_key.to_string(),
        correlation_id: None,
        causation_id: None,
        uom_id: None,
        lot_code: None,
        serial_codes: None,
    }
}

// ============================================================================
// Helper: read on-hand row for a (tenant, item, warehouse, location_id)
// ============================================================================

#[derive(sqlx::FromRow, Debug)]
struct OnHandRow {
    quantity_on_hand: i64,
    available_status_on_hand: i64,
    location_id: Option<Uuid>,
}

async fn get_on_hand(
    pool: &sqlx::PgPool,
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
    location_id: Option<Uuid>,
) -> Option<OnHandRow> {
    match location_id {
        None => sqlx::query_as::<_, OnHandRow>(
            r#"
                SELECT quantity_on_hand, available_status_on_hand, location_id
                FROM item_on_hand
                WHERE tenant_id = $1 AND item_id = $2 AND warehouse_id = $3
                  AND location_id IS NULL
                "#,
        )
        .bind(tenant_id)
        .bind(item_id)
        .bind(warehouse_id)
        .fetch_optional(pool)
        .await
        .unwrap(),
        Some(loc_id) => sqlx::query_as::<_, OnHandRow>(
            r#"
                SELECT quantity_on_hand, available_status_on_hand, location_id
                FROM item_on_hand
                WHERE tenant_id = $1 AND item_id = $2 AND warehouse_id = $3
                  AND location_id = $4
                "#,
        )
        .bind(tenant_id)
        .bind(item_id)
        .bind(warehouse_id)
        .bind(loc_id)
        .fetch_optional(pool)
        .await
        .unwrap(),
    }
}

// ============================================================================
// 1. Location CRUD happy path
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_and_get_location() {
    let pool = setup_db().await;
    let tenant_id = format!("loc-test-{}", Uuid::new_v4().simple());
    let warehouse_id = Uuid::new_v4();

    let req = CreateLocationRequest {
        tenant_id: tenant_id.clone(),
        warehouse_id,
        code: "BIN-A1".to_string(),
        name: "Bin A1 (front)".to_string(),
        description: Some("Front-of-warehouse bin".to_string()),
    };

    let loc = LocationRepo::create(&pool, &req).await.unwrap();
    assert_eq!(loc.code, "BIN-A1");
    assert_eq!(loc.name, "Bin A1 (front)");
    assert!(loc.is_active);

    let fetched = LocationRepo::find_by_id(&pool, loc.id, &tenant_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fetched.id, loc.id);
    assert_eq!(fetched.warehouse_id, warehouse_id);
}

// ============================================================================
// 2. Duplicate code rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn test_duplicate_location_code_rejected() {
    let pool = setup_db().await;
    let tenant_id = format!("loc-dup-{}", Uuid::new_v4().simple());
    let warehouse_id = Uuid::new_v4();

    let req = CreateLocationRequest {
        tenant_id: tenant_id.clone(),
        warehouse_id,
        code: "SHELF-01".to_string(),
        name: "Shelf 01".to_string(),
        description: None,
    };

    LocationRepo::create(&pool, &req).await.unwrap();

    let dup = CreateLocationRequest {
        tenant_id: tenant_id.clone(),
        warehouse_id,
        code: "SHELF-01".to_string(),
        name: "Shelf 01 duplicate".to_string(),
        description: None,
    };
    let err = LocationRepo::create(&pool, &dup).await.unwrap_err();
    assert!(matches!(err, LocationError::DuplicateCode(_, _, _)));
}

// ============================================================================
// 3. Update and deactivate
// ============================================================================

#[tokio::test]
#[serial]
async fn test_update_and_deactivate_location() {
    let pool = setup_db().await;
    let tenant_id = format!("loc-upd-{}", Uuid::new_v4().simple());
    let warehouse_id = Uuid::new_v4();

    let loc = LocationRepo::create(
        &pool,
        &CreateLocationRequest {
            tenant_id: tenant_id.clone(),
            warehouse_id,
            code: "ZONE-C".to_string(),
            name: "Zone C".to_string(),
            description: None,
        },
    )
    .await
    .unwrap();

    let updated = LocationRepo::update(
        &pool,
        loc.id,
        &UpdateLocationRequest {
            tenant_id: tenant_id.clone(),
            name: Some("Zone C (cold storage)".to_string()),
            description: Some("Refrigerated zone".to_string()),
        },
    )
    .await
    .unwrap();
    assert_eq!(updated.name, "Zone C (cold storage)");
    assert_eq!(updated.description, Some("Refrigerated zone".to_string()));

    let deactivated = LocationRepo::deactivate(&pool, loc.id, &tenant_id)
        .await
        .unwrap();
    assert!(!deactivated.is_active);
}

// ============================================================================
// 4. List locations for warehouse
// ============================================================================

#[tokio::test]
#[serial]
async fn test_list_locations_for_warehouse() {
    let pool = setup_db().await;
    let tenant_id = format!("loc-list-{}", Uuid::new_v4().simple());
    let warehouse_id = Uuid::new_v4();
    let other_warehouse = Uuid::new_v4();

    for code in &["BIN-1", "BIN-2", "BIN-3"] {
        LocationRepo::create(
            &pool,
            &CreateLocationRequest {
                tenant_id: tenant_id.clone(),
                warehouse_id,
                code: code.to_string(),
                name: format!("Bin {}", code),
                description: None,
            },
        )
        .await
        .unwrap();
    }
    // Create one in a different warehouse — should NOT appear
    LocationRepo::create(
        &pool,
        &CreateLocationRequest {
            tenant_id: tenant_id.clone(),
            warehouse_id: other_warehouse,
            code: "OTHER-BIN".to_string(),
            name: "Other bin".to_string(),
            description: None,
        },
    )
    .await
    .unwrap();

    let list = LocationRepo::list_for_warehouse(&pool, &tenant_id, warehouse_id)
        .await
        .unwrap();
    assert_eq!(list.len(), 3);
    assert_eq!(list[0].code, "BIN-1");
    assert_eq!(list[2].code, "BIN-3");
}

// ============================================================================
// 5. Receipt with location_id → on_hand row is location-scoped
// ============================================================================

#[tokio::test]
#[serial]
async fn test_receipt_with_location_creates_location_on_hand_row() {
    let pool = setup_db().await;
    let tenant_id = format!("loc-rcpt-{}", Uuid::new_v4().simple());
    let warehouse_id = Uuid::new_v4();

    // Create item
    let item = ItemRepo::create(&pool, &item_req(&tenant_id, "SKU-LOC-001"))
        .await
        .unwrap();

    // Create location
    let loc = LocationRepo::create(
        &pool,
        &CreateLocationRequest {
            tenant_id: tenant_id.clone(),
            warehouse_id,
            code: "BIN-X1".to_string(),
            name: "Bin X1".to_string(),
            description: None,
        },
    )
    .await
    .unwrap();

    // Receipt into location
    let req = receipt_req(
        &tenant_id,
        item.id,
        warehouse_id,
        Some(loc.id),
        "idem-loc-rcpt-1",
    );
    let (result, is_replay) = process_receipt(&pool, &req, None).await.unwrap();
    assert!(!is_replay);
    assert_eq!(result.location_id, Some(loc.id));

    // Check location-specific on_hand row was created
    let on_hand = get_on_hand(&pool, &tenant_id, item.id, warehouse_id, Some(loc.id))
        .await
        .unwrap();
    assert_eq!(on_hand.quantity_on_hand, 50);
    assert_eq!(on_hand.available_status_on_hand, 50);
    assert_eq!(on_hand.location_id, Some(loc.id));

    // Verify NO null-location row was created
    let null_row = get_on_hand(&pool, &tenant_id, item.id, warehouse_id, None).await;
    assert!(
        null_row.is_none(),
        "null-location row should not exist when location is specified"
    );

    // Check ledger row has location_id
    let ledger_loc: Option<Uuid> =
        sqlx::query_scalar("SELECT location_id FROM inventory_ledger WHERE source_event_id = $1")
            .bind(result.event_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(ledger_loc, Some(loc.id));
}

// ============================================================================
// 6. Receipt without location_id → null-location row (backward compat)
// ============================================================================

#[tokio::test]
#[serial]
async fn test_receipt_without_location_creates_null_location_row() {
    let pool = setup_db().await;
    let tenant_id = format!("loc-null-rcpt-{}", Uuid::new_v4().simple());
    let warehouse_id = Uuid::new_v4();

    let item = ItemRepo::create(&pool, &item_req(&tenant_id, "SKU-NULL-LOC-001"))
        .await
        .unwrap();

    let req = receipt_req(&tenant_id, item.id, warehouse_id, None, "idem-null-rcpt-1");
    let (result, _) = process_receipt(&pool, &req, None).await.unwrap();
    assert_eq!(result.location_id, None);

    let on_hand = get_on_hand(&pool, &tenant_id, item.id, warehouse_id, None)
        .await
        .unwrap();
    assert_eq!(on_hand.quantity_on_hand, 50);
    assert!(on_hand.location_id.is_none());
}

// ============================================================================
// 7. Separate on-hand rows per location (two receipts, two locations)
// ============================================================================

#[tokio::test]
#[serial]
async fn test_receipts_into_different_locations_create_separate_on_hand_rows() {
    let pool = setup_db().await;
    let tenant_id = format!("loc-sep-{}", Uuid::new_v4().simple());
    let warehouse_id = Uuid::new_v4();

    let item = ItemRepo::create(&pool, &item_req(&tenant_id, "SKU-SEP-LOC"))
        .await
        .unwrap();

    let loc_a = LocationRepo::create(
        &pool,
        &CreateLocationRequest {
            tenant_id: tenant_id.clone(),
            warehouse_id,
            code: "A-SHELF".to_string(),
            name: "Shelf A".to_string(),
            description: None,
        },
    )
    .await
    .unwrap();

    let loc_b = LocationRepo::create(
        &pool,
        &CreateLocationRequest {
            tenant_id: tenant_id.clone(),
            warehouse_id,
            code: "B-SHELF".to_string(),
            name: "Shelf B".to_string(),
            description: None,
        },
    )
    .await
    .unwrap();

    // 50 into loc_a, 30 into loc_b
    process_receipt(
        &pool,
        &receipt_req(&tenant_id, item.id, warehouse_id, Some(loc_a.id), "idem-a"),
        None,
    )
    .await
    .unwrap();

    let mut req_b = receipt_req(&tenant_id, item.id, warehouse_id, Some(loc_b.id), "idem-b");
    req_b.quantity = 30;
    process_receipt(&pool, &req_b, None).await.unwrap();

    let on_hand_a = get_on_hand(&pool, &tenant_id, item.id, warehouse_id, Some(loc_a.id))
        .await
        .unwrap();
    let on_hand_b = get_on_hand(&pool, &tenant_id, item.id, warehouse_id, Some(loc_b.id))
        .await
        .unwrap();

    assert_eq!(on_hand_a.quantity_on_hand, 50, "location A should have 50");
    assert_eq!(on_hand_b.quantity_on_hand, 30, "location B should have 30");

    // Rows are separate
    assert_ne!(on_hand_a.location_id, on_hand_b.location_id);
}

// ============================================================================
// 8. Issue with location_id → deducts from location-specific on_hand
// ============================================================================

#[tokio::test]
#[serial]
async fn test_issue_with_location_deducts_from_location_on_hand() {
    let pool = setup_db().await;
    let tenant_id = format!("loc-issue-{}", Uuid::new_v4().simple());
    let warehouse_id = Uuid::new_v4();

    let item = ItemRepo::create(&pool, &item_req(&tenant_id, "SKU-LOC-ISSUE"))
        .await
        .unwrap();

    let loc = LocationRepo::create(
        &pool,
        &CreateLocationRequest {
            tenant_id: tenant_id.clone(),
            warehouse_id,
            code: "ISSUE-BIN".to_string(),
            name: "Issue Bin".to_string(),
            description: None,
        },
    )
    .await
    .unwrap();

    // Receive 50 into location
    process_receipt(
        &pool,
        &receipt_req(
            &tenant_id,
            item.id,
            warehouse_id,
            Some(loc.id),
            "idem-issue-rcpt",
        ),
        None,
    )
    .await
    .unwrap();

    // Issue 15 from location
    let issue = issue_req(
        &tenant_id,
        item.id,
        warehouse_id,
        Some(loc.id),
        15,
        "idem-issue-1",
    );
    let (result, is_replay) = process_issue(&pool, &issue, None).await.unwrap();
    assert!(!is_replay);
    assert_eq!(result.location_id, Some(loc.id));
    assert_eq!(result.quantity, 15);

    let on_hand = get_on_hand(&pool, &tenant_id, item.id, warehouse_id, Some(loc.id))
        .await
        .unwrap();
    assert_eq!(on_hand.quantity_on_hand, 35, "50 - 15 = 35");
    assert_eq!(on_hand.available_status_on_hand, 35);
}

// ============================================================================
// 9. Issue with location_id insufficient stock
// ============================================================================

#[tokio::test]
#[serial]
async fn test_issue_from_location_rejects_insufficient_stock() {
    let pool = setup_db().await;
    let tenant_id = format!("loc-insuf-{}", Uuid::new_v4().simple());
    let warehouse_id = Uuid::new_v4();

    let item = ItemRepo::create(&pool, &item_req(&tenant_id, "SKU-INSUF-LOC"))
        .await
        .unwrap();

    let loc = LocationRepo::create(
        &pool,
        &CreateLocationRequest {
            tenant_id: tenant_id.clone(),
            warehouse_id,
            code: "INSUF-BIN".to_string(),
            name: "Insufficient Bin".to_string(),
            description: None,
        },
    )
    .await
    .unwrap();

    // Only 10 in location
    let mut rcpt = receipt_req(
        &tenant_id,
        item.id,
        warehouse_id,
        Some(loc.id),
        "idem-insuf-rcpt",
    );
    rcpt.quantity = 10;
    process_receipt(&pool, &rcpt, None).await.unwrap();

    // Try to issue 20 — should fail
    let issue = issue_req(
        &tenant_id,
        item.id,
        warehouse_id,
        Some(loc.id),
        20,
        "idem-insuf-issue",
    );
    let err = process_issue(&pool, &issue, None).await.unwrap_err();
    assert!(
        matches!(
            err,
            inventory_rs::domain::issue_service::IssueError::InsufficientQuantity {
                requested: 20,
                available: 10
            }
        ),
        "expected InsufficientQuantity, got: {:?}",
        err
    );
}

// ============================================================================
// 10. Null-location issue still works (backward compat)
// ============================================================================

#[tokio::test]
#[serial]
async fn test_null_location_issue_backward_compat() {
    let pool = setup_db().await;
    let tenant_id = format!("loc-back-{}", Uuid::new_v4().simple());
    let warehouse_id = Uuid::new_v4();

    let item = ItemRepo::create(&pool, &item_req(&tenant_id, "SKU-BACK-COMPAT"))
        .await
        .unwrap();

    // Receipt with no location
    process_receipt(
        &pool,
        &receipt_req(&tenant_id, item.id, warehouse_id, None, "idem-back-rcpt"),
        None,
    )
    .await
    .unwrap();

    // Issue with no location
    let issue = issue_req(
        &tenant_id,
        item.id,
        warehouse_id,
        None,
        20,
        "idem-back-issue",
    );
    let (result, _) = process_issue(&pool, &issue, None).await.unwrap();
    assert_eq!(result.location_id, None);
    assert_eq!(result.quantity, 20);

    let on_hand = get_on_hand(&pool, &tenant_id, item.id, warehouse_id, None)
        .await
        .unwrap();
    assert_eq!(on_hand.quantity_on_hand, 30, "50 - 20 = 30");
}
