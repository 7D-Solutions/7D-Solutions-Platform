//! Integration tests for ItemRepo::list (search, filter, pagination).
//!
//! Tests run against a real PostgreSQL database.
//! Set DATABASE_URL to the inventory database connection string.

use inventory_rs::domain::items::{CreateItemRequest, ItemRepo, ListItemsQuery, TrackingMode};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

// ============================================================================
// Helpers
// ============================================================================

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://inventory_user:inventory_pass@localhost:5442/inventory_db?sslmode=disable"
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

fn item_req(tenant: &str, sku: &str, name: &str) -> CreateItemRequest {
    CreateItemRequest {
        tenant_id: tenant.to_string(),
        sku: sku.to_string(),
        name: name.to_string(),
        description: None,
        inventory_account_ref: "1200".to_string(),
        cogs_account_ref: "5000".to_string(),
        variance_account_ref: "5010".to_string(),
        uom: None,
        tracking_mode: TrackingMode::None,
        make_buy: None,
    }
}

fn default_query() -> ListItemsQuery {
    ListItemsQuery {
        search: None,
        tracking_mode: None,
        make_buy: None,
        active: None,
        limit: 50,
        offset: 0,
    }
}

async fn cleanup(pool: &sqlx::PgPool, tenant: &str) {
    sqlx::query("DELETE FROM items WHERE tenant_id = $1")
        .bind(tenant)
        .execute(pool)
        .await
        .ok();
}

// ============================================================================
// Tests
// ============================================================================

#[tokio::test]
#[serial]
async fn list_empty_returns_zero() {
    let pool = setup_db().await;
    let tenant = format!("list-empty-{}", Uuid::new_v4());
    cleanup(&pool, &tenant).await;

    let (items, total) = ItemRepo::list(&pool, &tenant, &default_query())
        .await
        .unwrap();
    assert!(items.is_empty());
    assert_eq!(total, 0);

    cleanup(&pool, &tenant).await;
}

#[tokio::test]
#[serial]
async fn list_returns_items_ordered_by_name() {
    let pool = setup_db().await;
    let tenant = format!("list-order-{}", Uuid::new_v4());
    cleanup(&pool, &tenant).await;

    ItemRepo::create(&pool, &item_req(&tenant, "SKU-B", "Banana"))
        .await
        .unwrap();
    ItemRepo::create(&pool, &item_req(&tenant, "SKU-A", "Apple"))
        .await
        .unwrap();
    ItemRepo::create(&pool, &item_req(&tenant, "SKU-C", "Cherry"))
        .await
        .unwrap();

    let (items, total) = ItemRepo::list(&pool, &tenant, &default_query())
        .await
        .unwrap();
    assert_eq!(total, 3);
    assert_eq!(items.len(), 3);
    assert_eq!(items[0].name, "Apple");
    assert_eq!(items[1].name, "Banana");
    assert_eq!(items[2].name, "Cherry");

    cleanup(&pool, &tenant).await;
}

#[tokio::test]
#[serial]
async fn search_by_sku_substring() {
    let pool = setup_db().await;
    let tenant = format!("list-search-{}", Uuid::new_v4());
    cleanup(&pool, &tenant).await;

    ItemRepo::create(&pool, &item_req(&tenant, "BOLT-M6", "Hex Bolt"))
        .await
        .unwrap();
    ItemRepo::create(&pool, &item_req(&tenant, "NUT-M6", "Hex Nut"))
        .await
        .unwrap();
    ItemRepo::create(&pool, &item_req(&tenant, "WASHER-M6", "Flat Washer"))
        .await
        .unwrap();

    let mut q = default_query();
    q.search = Some("BOLT".to_string());
    let (items, total) = ItemRepo::list(&pool, &tenant, &q).await.unwrap();
    assert_eq!(total, 1);
    assert_eq!(items[0].sku, "BOLT-M6");

    cleanup(&pool, &tenant).await;
}

#[tokio::test]
#[serial]
async fn filter_by_tracking_mode() {
    let pool = setup_db().await;
    let tenant = format!("list-track-{}", Uuid::new_v4());
    cleanup(&pool, &tenant).await;

    let mut lot_req = item_req(&tenant, "LOT-001", "Lot Item");
    lot_req.tracking_mode = TrackingMode::Lot;
    ItemRepo::create(&pool, &lot_req).await.unwrap();

    ItemRepo::create(&pool, &item_req(&tenant, "NONE-001", "None Item"))
        .await
        .unwrap();

    let mut q = default_query();
    q.tracking_mode = Some("lot".to_string());
    let (items, total) = ItemRepo::list(&pool, &tenant, &q).await.unwrap();
    assert_eq!(total, 1);
    assert_eq!(items[0].sku, "LOT-001");

    cleanup(&pool, &tenant).await;
}

#[tokio::test]
#[serial]
async fn default_excludes_deactivated() {
    let pool = setup_db().await;
    let tenant = format!("list-active-{}", Uuid::new_v4());
    cleanup(&pool, &tenant).await;

    let active = ItemRepo::create(&pool, &item_req(&tenant, "ACTIVE-1", "Active Item"))
        .await
        .unwrap();
    let inactive = ItemRepo::create(&pool, &item_req(&tenant, "INACTIVE-1", "Inactive Item"))
        .await
        .unwrap();
    ItemRepo::deactivate(&pool, inactive.id, &tenant)
        .await
        .unwrap();

    let (items, total) = ItemRepo::list(&pool, &tenant, &default_query())
        .await
        .unwrap();
    assert_eq!(total, 1);
    assert_eq!(items[0].id, active.id);

    cleanup(&pool, &tenant).await;
}

#[tokio::test]
#[serial]
async fn active_false_returns_only_inactive() {
    let pool = setup_db().await;
    let tenant = format!("list-inactive-{}", Uuid::new_v4());
    cleanup(&pool, &tenant).await;

    ItemRepo::create(&pool, &item_req(&tenant, "ACTIVE-2", "Active"))
        .await
        .unwrap();
    let inactive = ItemRepo::create(&pool, &item_req(&tenant, "INACTIVE-2", "Inactive"))
        .await
        .unwrap();
    ItemRepo::deactivate(&pool, inactive.id, &tenant)
        .await
        .unwrap();

    let mut q = default_query();
    q.active = Some(false);
    let (items, total) = ItemRepo::list(&pool, &tenant, &q).await.unwrap();
    assert_eq!(total, 1);
    assert_eq!(items[0].sku, "INACTIVE-2");

    cleanup(&pool, &tenant).await;
}

#[tokio::test]
#[serial]
async fn pagination_limit_offset() {
    let pool = setup_db().await;
    let tenant = format!("list-page-{}", Uuid::new_v4());
    cleanup(&pool, &tenant).await;

    for (sku, name) in [
        ("S-A", "A"),
        ("S-B", "B"),
        ("S-C", "C"),
        ("S-D", "D"),
        ("S-E", "E"),
    ] {
        ItemRepo::create(&pool, &item_req(&tenant, sku, name))
            .await
            .unwrap();
    }

    let mut q = default_query();
    q.limit = 2;
    q.offset = 0;
    let (page1, total) = ItemRepo::list(&pool, &tenant, &q).await.unwrap();
    assert_eq!(total, 5);
    assert_eq!(page1.len(), 2);
    assert_eq!(page1[0].name, "A");
    assert_eq!(page1[1].name, "B");

    q.offset = 2;
    let (page2, _) = ItemRepo::list(&pool, &tenant, &q).await.unwrap();
    assert_eq!(page2.len(), 2);
    assert_eq!(page2[0].name, "C");
    assert_eq!(page2[1].name, "D");

    q.offset = 4;
    let (page3, _) = ItemRepo::list(&pool, &tenant, &q).await.unwrap();
    assert_eq!(page3.len(), 1);
    assert_eq!(page3[0].name, "E");

    cleanup(&pool, &tenant).await;
}

#[tokio::test]
#[serial]
async fn tenant_isolation() {
    let pool = setup_db().await;
    let tenant_a = format!("list-iso-a-{}", Uuid::new_v4());
    let tenant_b = format!("list-iso-b-{}", Uuid::new_v4());
    cleanup(&pool, &tenant_a).await;
    cleanup(&pool, &tenant_b).await;

    ItemRepo::create(&pool, &item_req(&tenant_a, "A-ITEM", "A Item"))
        .await
        .unwrap();
    ItemRepo::create(&pool, &item_req(&tenant_b, "B-ITEM", "B Item"))
        .await
        .unwrap();

    let (items_a, total_a) = ItemRepo::list(&pool, &tenant_a, &default_query())
        .await
        .unwrap();
    assert_eq!(total_a, 1);
    assert_eq!(items_a[0].sku, "A-ITEM");

    let (items_b, total_b) = ItemRepo::list(&pool, &tenant_b, &default_query())
        .await
        .unwrap();
    assert_eq!(total_b, 1);
    assert_eq!(items_b[0].sku, "B-ITEM");

    cleanup(&pool, &tenant_a).await;
    cleanup(&pool, &tenant_b).await;
}
