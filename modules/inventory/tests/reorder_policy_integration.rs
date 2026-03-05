//! Integration tests for reorder policies (bd-1z9v).
//!
//! Covers:
//! 1. Create policy (global — no location)
//! 2. Create policy (location-scoped)
//! 3. Duplicate (item, NULL location) rejected with DuplicatePolicy
//! 4. Duplicate (item, same location) rejected with DuplicatePolicy
//! 5. Update policy thresholds
//! 6. Get policy by id
//! 7. List policies for item — global first, then location-scoped
//! 8. Negative reorder_point rejected at domain layer

use inventory_rs::domain::{
    items::{CreateItemRequest, ItemRepo, TrackingMode},
    locations::{CreateLocationRequest, LocationRepo},
    reorder::models::{
        CreateReorderPolicyRequest, ReorderPolicyError, ReorderPolicyRepo,
        UpdateReorderPolicyRequest,
    },
};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

// ============================================================================
// Test DB helpers
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

fn item_req(tenant_id: &str, sku: &str) -> CreateItemRequest {
    CreateItemRequest {
        tenant_id: tenant_id.to_string(),
        sku: sku.to_string(),
        name: "Widget".to_string(),
        description: None,
        inventory_account_ref: "1200".to_string(),
        cogs_account_ref: "5000".to_string(),
        variance_account_ref: "5010".to_string(),
        uom: None,
        tracking_mode: TrackingMode::None,
        make_buy: None,
    }
}

fn loc_req(tenant_id: &str, warehouse_id: Uuid, code: &str) -> CreateLocationRequest {
    CreateLocationRequest {
        tenant_id: tenant_id.to_string(),
        warehouse_id,
        code: code.to_string(),
        name: format!("Location {}", code),
        description: None,
    }
}

// ============================================================================
// Tests
// ============================================================================

/// 1. Create a global policy (location_id = NULL)
#[tokio::test]
#[serial]
async fn test_create_global_reorder_policy() {
    let pool = setup_db().await;
    let tenant_id = format!("t-{}", Uuid::new_v4());
    let item = ItemRepo::create(&pool, &item_req(&tenant_id, "SKU-REORDER-1"))
        .await
        .unwrap();

    let req = CreateReorderPolicyRequest {
        tenant_id: tenant_id.clone(),
        item_id: item.id,
        location_id: None,
        reorder_point: 50,
        safety_stock: 10,
        max_qty: Some(200),
        notes: Some("Weekly restock".into()),
        created_by: Some("alice".into()),
    };
    let policy = ReorderPolicyRepo::create(&pool, &req).await.unwrap();

    assert_eq!(policy.tenant_id, tenant_id);
    assert_eq!(policy.item_id, item.id);
    assert_eq!(policy.location_id, None);
    assert_eq!(policy.reorder_point, 50);
    assert_eq!(policy.safety_stock, 10);
    assert_eq!(policy.max_qty, Some(200));
    assert_eq!(policy.created_by, "alice");
}

/// 2. Create a location-scoped policy
#[tokio::test]
#[serial]
async fn test_create_location_scoped_reorder_policy() {
    let pool = setup_db().await;
    let tenant_id = format!("t-{}", Uuid::new_v4());
    let warehouse_id = Uuid::new_v4();

    let item = ItemRepo::create(&pool, &item_req(&tenant_id, "SKU-REORDER-2"))
        .await
        .unwrap();
    let loc = LocationRepo::create(&pool, &loc_req(&tenant_id, warehouse_id, "BIN-A1"))
        .await
        .unwrap();

    let req = CreateReorderPolicyRequest {
        tenant_id: tenant_id.clone(),
        item_id: item.id,
        location_id: Some(loc.id),
        reorder_point: 20,
        safety_stock: 5,
        max_qty: None,
        notes: None,
        created_by: None,
    };
    let policy = ReorderPolicyRepo::create(&pool, &req).await.unwrap();

    assert_eq!(policy.location_id, Some(loc.id));
    assert_eq!(policy.reorder_point, 20);
    assert_eq!(policy.safety_stock, 5);
    assert_eq!(policy.max_qty, None);
    assert_eq!(policy.created_by, "system");
}

/// 3. Duplicate global policy (item, NULL location) is rejected
#[tokio::test]
#[serial]
async fn test_duplicate_global_policy_rejected() {
    let pool = setup_db().await;
    let tenant_id = format!("t-{}", Uuid::new_v4());
    let item = ItemRepo::create(&pool, &item_req(&tenant_id, "SKU-REORDER-3"))
        .await
        .unwrap();

    let req = CreateReorderPolicyRequest {
        tenant_id: tenant_id.clone(),
        item_id: item.id,
        location_id: None,
        reorder_point: 30,
        safety_stock: 5,
        max_qty: None,
        notes: None,
        created_by: None,
    };
    ReorderPolicyRepo::create(&pool, &req).await.unwrap();

    let req2 = CreateReorderPolicyRequest {
        tenant_id: tenant_id.clone(),
        item_id: item.id,
        location_id: None,
        reorder_point: 40,
        safety_stock: 8,
        max_qty: None,
        notes: None,
        created_by: None,
    };
    let err = ReorderPolicyRepo::create(&pool, &req2).await.unwrap_err();
    assert!(
        matches!(err, ReorderPolicyError::DuplicatePolicy),
        "expected DuplicatePolicy, got {:?}",
        err
    );
}

/// 4. Duplicate location-scoped policy (item, same location) is rejected
#[tokio::test]
#[serial]
async fn test_duplicate_location_policy_rejected() {
    let pool = setup_db().await;
    let tenant_id = format!("t-{}", Uuid::new_v4());
    let warehouse_id = Uuid::new_v4();

    let item = ItemRepo::create(&pool, &item_req(&tenant_id, "SKU-REORDER-4"))
        .await
        .unwrap();
    let loc = LocationRepo::create(&pool, &loc_req(&tenant_id, warehouse_id, "BIN-DUP"))
        .await
        .unwrap();

    let make_req = |rp: i64| CreateReorderPolicyRequest {
        tenant_id: tenant_id.clone(),
        item_id: item.id,
        location_id: Some(loc.id),
        reorder_point: rp,
        safety_stock: 5,
        max_qty: None,
        notes: None,
        created_by: None,
    };

    ReorderPolicyRepo::create(&pool, &make_req(10))
        .await
        .unwrap();
    let err = ReorderPolicyRepo::create(&pool, &make_req(20))
        .await
        .unwrap_err();
    assert!(
        matches!(err, ReorderPolicyError::DuplicatePolicy),
        "expected DuplicatePolicy, got {:?}",
        err
    );
}

/// 5. Update policy thresholds
#[tokio::test]
#[serial]
async fn test_update_reorder_policy() {
    let pool = setup_db().await;
    let tenant_id = format!("t-{}", Uuid::new_v4());
    let item = ItemRepo::create(&pool, &item_req(&tenant_id, "SKU-REORDER-5"))
        .await
        .unwrap();

    let create_req = CreateReorderPolicyRequest {
        tenant_id: tenant_id.clone(),
        item_id: item.id,
        location_id: None,
        reorder_point: 50,
        safety_stock: 10,
        max_qty: None,
        notes: None,
        created_by: Some("alice".into()),
    };
    let policy = ReorderPolicyRepo::create(&pool, &create_req).await.unwrap();

    let update_req = UpdateReorderPolicyRequest {
        tenant_id: tenant_id.clone(),
        reorder_point: Some(75),
        safety_stock: Some(15),
        max_qty: Some(300),
        notes: Some("Updated note".into()),
        updated_by: Some("bob".into()),
    };
    let updated = ReorderPolicyRepo::update(&pool, policy.id, &update_req)
        .await
        .unwrap();

    assert_eq!(updated.reorder_point, 75);
    assert_eq!(updated.safety_stock, 15);
    assert_eq!(updated.max_qty, Some(300));
    assert_eq!(updated.notes.as_deref(), Some("Updated note"));
    assert_eq!(updated.updated_by, "bob");
    assert!(updated.updated_at >= policy.updated_at);
}

/// 6. Get policy by id
#[tokio::test]
#[serial]
async fn test_get_reorder_policy_by_id() {
    let pool = setup_db().await;
    let tenant_id = format!("t-{}", Uuid::new_v4());
    let item = ItemRepo::create(&pool, &item_req(&tenant_id, "SKU-REORDER-6"))
        .await
        .unwrap();

    let req = CreateReorderPolicyRequest {
        tenant_id: tenant_id.clone(),
        item_id: item.id,
        location_id: None,
        reorder_point: 60,
        safety_stock: 12,
        max_qty: None,
        notes: None,
        created_by: None,
    };
    let created = ReorderPolicyRepo::create(&pool, &req).await.unwrap();

    // Get by correct id + tenant
    let found = ReorderPolicyRepo::find_by_id(&pool, created.id, &tenant_id)
        .await
        .unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().id, created.id);

    // Wrong tenant → None
    let not_found = ReorderPolicyRepo::find_by_id(&pool, created.id, "wrong-tenant")
        .await
        .unwrap();
    assert!(not_found.is_none());
}

/// 7. List policies for item: global first, then location-scoped
#[tokio::test]
#[serial]
async fn test_list_reorder_policies_for_item() {
    let pool = setup_db().await;
    let tenant_id = format!("t-{}", Uuid::new_v4());
    let warehouse_id = Uuid::new_v4();

    let item = ItemRepo::create(&pool, &item_req(&tenant_id, "SKU-REORDER-7"))
        .await
        .unwrap();
    let loc1 = LocationRepo::create(&pool, &loc_req(&tenant_id, warehouse_id, "BIN-L1"))
        .await
        .unwrap();
    let loc2 = LocationRepo::create(&pool, &loc_req(&tenant_id, warehouse_id, "BIN-L2"))
        .await
        .unwrap();

    // Create: location-scoped first, then global (to verify ordering)
    ReorderPolicyRepo::create(
        &pool,
        &CreateReorderPolicyRequest {
            tenant_id: tenant_id.clone(),
            item_id: item.id,
            location_id: Some(loc1.id),
            reorder_point: 20,
            safety_stock: 5,
            max_qty: None,
            notes: None,
            created_by: None,
        },
    )
    .await
    .unwrap();

    ReorderPolicyRepo::create(
        &pool,
        &CreateReorderPolicyRequest {
            tenant_id: tenant_id.clone(),
            item_id: item.id,
            location_id: None,
            reorder_point: 100,
            safety_stock: 25,
            max_qty: None,
            notes: None,
            created_by: None,
        },
    )
    .await
    .unwrap();

    ReorderPolicyRepo::create(
        &pool,
        &CreateReorderPolicyRequest {
            tenant_id: tenant_id.clone(),
            item_id: item.id,
            location_id: Some(loc2.id),
            reorder_point: 30,
            safety_stock: 8,
            max_qty: None,
            notes: None,
            created_by: None,
        },
    )
    .await
    .unwrap();

    let policies = ReorderPolicyRepo::list_for_item(&pool, &tenant_id, item.id)
        .await
        .unwrap();

    assert_eq!(policies.len(), 3);
    // Global policy (location_id = NULL) must come first
    assert!(policies[0].location_id.is_none(), "first should be global");
    assert_eq!(policies[0].reorder_point, 100);
}

/// 8. Negative reorder_point rejected at domain layer (no DB round-trip needed)
#[tokio::test]
#[serial]
async fn test_negative_reorder_point_rejected() {
    let pool = setup_db().await;
    let tenant_id = format!("t-{}", Uuid::new_v4());
    let item = ItemRepo::create(&pool, &item_req(&tenant_id, "SKU-REORDER-8"))
        .await
        .unwrap();

    let req = CreateReorderPolicyRequest {
        tenant_id: tenant_id.clone(),
        item_id: item.id,
        location_id: None,
        reorder_point: -1,
        safety_stock: 5,
        max_qty: None,
        notes: None,
        created_by: None,
    };
    let err = ReorderPolicyRepo::create(&pool, &req).await.unwrap_err();
    assert!(
        matches!(err, ReorderPolicyError::Validation(_)),
        "expected Validation error, got {:?}",
        err
    );
}
