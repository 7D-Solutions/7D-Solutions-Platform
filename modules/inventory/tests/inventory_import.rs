//! Integration tests for POST /api/inventory/import/items.
//!
//! Connects to a real PostgreSQL database.  No mocks, no stubs.
//! Requires DATABASE_URL env var (or falls back to the default Inventory dev URL).
//!
//! Run with:
//!   ./scripts/cargo-slot.sh test -p inventory-rs --test inventory_import -- --nocapture

use inventory_rs::http::imports::{run_items_import, ItemImportRow};
use serial_test::serial;
use sqlx::{postgres::PgPoolOptions, PgPool};
use uuid::Uuid;

async fn setup_pool() -> PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://inventory_user:inventory_pass@localhost:5442/inventory_db".to_string());
    PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to inventory DB")
}

fn unique_tenant() -> String {
    format!("import-inv-{}", Uuid::new_v4().simple())
}

async fn cleanup(pool: &PgPool, tenant_id: &str) {
    sqlx::query("DELETE FROM items WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
}

fn item(code: &str, name: &str) -> ItemImportRow {
    ItemImportRow {
        item_code: code.into(),
        name: name.into(),
        unit_of_measure: Some("ea".into()),
        tracking_mode: Some("none".into()),
        inventory_account_ref: Some("1200".into()),
        cogs_account_ref: Some("5000".into()),
        variance_account_ref: Some("5010".into()),
        reorder_point: None,
    }
}

// ============================================================================
// Tests
// ============================================================================

#[tokio::test]
#[serial]
async fn inventory_import_creates_new_items() {
    let pool = setup_pool().await;
    let tenant = unique_tenant();
    cleanup(&pool, &tenant).await;

    let rows = vec![item("SKU-001", "Widget A"), item("SKU-002", "Widget B")];

    let summary = run_items_import(&pool, &tenant, &rows)
        .await
        .expect("import should succeed");

    assert_eq!(summary.created, 2);
    assert_eq!(summary.updated, 0);
    assert_eq!(summary.skipped, 0);
    assert!(summary.errors.is_empty());

    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM items WHERE tenant_id = $1")
            .bind(&tenant)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 2);

    cleanup(&pool, &tenant).await;
}

#[tokio::test]
#[serial]
async fn inventory_import_skips_identical_on_reimport() {
    let pool = setup_pool().await;
    let tenant = unique_tenant();
    cleanup(&pool, &tenant).await;

    let rows = vec![item("SKU-001", "Widget A")];

    let s1 = run_items_import(&pool, &tenant, &rows).await.unwrap();
    assert_eq!(s1.created, 1);

    let s2 = run_items_import(&pool, &tenant, &rows).await.unwrap();
    assert_eq!(s2.created, 0);
    assert_eq!(s2.updated, 0);
    assert_eq!(s2.skipped, 1);
    assert!(s2.errors.is_empty());

    cleanup(&pool, &tenant).await;
}

#[tokio::test]
#[serial]
async fn inventory_import_updates_changed_name() {
    let pool = setup_pool().await;
    let tenant = unique_tenant();
    cleanup(&pool, &tenant).await;

    run_items_import(&pool, &tenant, &[item("SKU-001", "Old Name")])
        .await
        .unwrap();

    let mut updated = item("SKU-001", "New Name");
    let s2 = run_items_import(&pool, &tenant, &[updated]).await.unwrap();

    assert_eq!(s2.updated, 1);
    assert_eq!(s2.skipped, 0);
    assert!(s2.errors.is_empty());

    let name: String =
        sqlx::query_scalar("SELECT name FROM items WHERE tenant_id = $1 AND sku = 'SKU-001'")
            .bind(&tenant)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(name, "New Name");

    cleanup(&pool, &tenant).await;
}

#[tokio::test]
#[serial]
async fn inventory_import_validates_all_before_insert() {
    let pool = setup_pool().await;
    let tenant = unique_tenant();
    cleanup(&pool, &tenant).await;

    let rows = vec![
        item("SKU-001", "Good Row"),
        ItemImportRow {
            item_code: "SKU-002".into(),
            name: "Bad tracking".into(),
            unit_of_measure: None,
            tracking_mode: Some("invalid".into()),
            inventory_account_ref: None,
            cogs_account_ref: None,
            variance_account_ref: None,
            reorder_point: None,
        },
    ];

    let summary = run_items_import(&pool, &tenant, &rows).await.unwrap();

    assert_eq!(summary.errors.len(), 1);
    assert_eq!(summary.errors[0].row, 2);
    assert_eq!(summary.created, 0, "No rows should be inserted when any row fails validation");

    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM items WHERE tenant_id = $1")
            .bind(&tenant)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 0);

    cleanup(&pool, &tenant).await;
}

#[tokio::test]
#[serial]
async fn inventory_import_defaults_uom_to_ea() {
    let pool = setup_pool().await;
    let tenant = unique_tenant();
    cleanup(&pool, &tenant).await;

    let rows = vec![ItemImportRow {
        item_code: "SKU-X".into(),
        name: "No UoM Row".into(),
        unit_of_measure: None,
        tracking_mode: None,
        inventory_account_ref: None,
        cogs_account_ref: None,
        variance_account_ref: None,
        reorder_point: None,
    }];

    let summary = run_items_import(&pool, &tenant, &rows).await.unwrap();
    assert_eq!(summary.created, 1);
    assert!(summary.errors.is_empty());

    let uom: String =
        sqlx::query_scalar("SELECT uom FROM items WHERE tenant_id = $1 AND sku = 'SKU-X'")
            .bind(&tenant)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(uom, "ea");

    cleanup(&pool, &tenant).await;
}
