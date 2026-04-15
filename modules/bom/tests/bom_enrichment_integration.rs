//! Integration tests for BOM line enrichment (`?include=item_details`).
//!
//! Tests verify:
//! 1. Without `include`, list_lines returns bare BomLine (backward compat).
//! 2. With `include=item_details`, each line has an embedded `item` object
//!    containing sku, name, description, and unit_cost_minor.
//! 3. An unresolvable `component_item_id` returns `item: null` — not an error.
//!
//! Requires two live databases:
//!   BOM_DATABASE_URL  (default: postgres://bom_user:bom_pass@localhost:5450/bom_db)
//!   INVENTORY_DATABASE_URL  (default: postgres://inventory_user:inventory_pass@localhost:5442/inventory_db)

use bom_rs::domain::bom_service::{self, list_lines_enriched};
use bom_rs::domain::inventory_client::InventoryClient;
use bom_rs::domain::models::{AddLineRequest, CreateBomRequest, CreateRevisionRequest};
use platform_sdk::PlatformClient;
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

async fn setup_bom_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("BOM_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| "postgres://bom_user:bom_pass@localhost:5450/bom_db".to_string());

    let pool = PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(&url)
        .await
        .expect("Failed to connect to BOM test DB");

    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run BOM migrations");

    pool
}

async fn setup_inventory_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("INVENTORY_DATABASE_URL").unwrap_or_else(|_| {
        "postgres://inventory_user:inventory_pass@localhost:5442/inventory_db".to_string()
    });

    PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(&url)
        .await
        .expect("Failed to connect to Inventory test DB")
}

fn dummy_claims() -> platform_sdk::VerifiedClaims {
    // Direct mode ignores claims; Uuid::nil() satisfies the type requirement.
    PlatformClient::service_claims(Uuid::nil())
}

fn unique_tenant() -> String {
    format!("test-tenant-{}", Uuid::new_v4())
}

/// Insert a minimal item into the inventory DB and return its id.
async fn insert_inventory_item(
    inv_pool: &sqlx::PgPool,
    tenant_id: &str,
    sku: &str,
    name: &str,
    description: Option<&str>,
) -> Uuid {
    let id: (Uuid,) = sqlx::query_as(
        r#"
        INSERT INTO items (tenant_id, sku, name, description,
                           inventory_account_ref, cogs_account_ref,
                           variance_account_ref, uom, active)
        VALUES ($1, $2, $3, $4, '1200', '5000', '5010', 'ea', true)
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(sku)
    .bind(name)
    .bind(description)
    .fetch_one(inv_pool)
    .await
    .expect("Failed to insert inventory item");
    id.0
}

/// Insert a standard-cost valuation config for an item.
async fn insert_standard_cost(
    inv_pool: &sqlx::PgPool,
    tenant_id: &str,
    item_id: Uuid,
    cost_minor: i64,
) {
    sqlx::query(
        r#"
        INSERT INTO item_valuation_configs (tenant_id, item_id, method, standard_cost_minor)
        VALUES ($1, $2, 'standard_cost', $3)
        ON CONFLICT (tenant_id, item_id) DO UPDATE SET standard_cost_minor = EXCLUDED.standard_cost_minor
        "#,
    )
    .bind(tenant_id)
    .bind(item_id)
    .bind(cost_minor)
    .execute(inv_pool)
    .await
    .expect("Failed to insert valuation config");
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Without `?include=item_details`, list_lines returns bare BomLine structs.
/// This test proves backward compatibility — no `item` field, no inventory call.
#[tokio::test]
#[serial]
async fn list_lines_without_include_returns_bare_lines() {
    let bom_pool = setup_bom_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let bom = bom_service::create_bom(
        &bom_pool,
        &tenant,
        &CreateBomRequest {
            part_id: Uuid::new_v4(),
            description: None,
        },
        &corr,
        None,
    )
    .await
    .expect("create_bom");

    let rev = bom_service::create_revision(
        &bom_pool,
        &tenant,
        bom.id,
        &CreateRevisionRequest {
            revision_label: "Rev-A".to_string(),
        },
        &corr,
        None,
    )
    .await
    .expect("create_revision");

    bom_service::add_line(
        &bom_pool,
        &tenant,
        rev.id,
        &AddLineRequest {
            component_item_id: Uuid::new_v4(),
            quantity: 2.0,
            uom: Some("EA".to_string()),
            scrap_factor: None,
            find_number: Some(10),
        },
        &corr,
        None,
    )
    .await
    .expect("add_line");

    let lines = bom_service::list_lines(&bom_pool, &tenant, rev.id)
        .await
        .expect("list_lines");

    assert_eq!(lines.len(), 1, "Expected 1 bare line");
    assert_eq!(lines[0].find_number, Some(10));
}

/// With `?include=item_details`, each line has an embedded item object with
/// sku, name, description, and unit_cost_minor (from standard_cost config).
#[tokio::test]
#[serial]
async fn list_lines_enriched_includes_item_details() {
    let bom_pool = setup_bom_db().await;
    let inv_pool = setup_inventory_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();
    let claims = dummy_claims();

    // Create an inventory item with a standard cost
    let item_id = insert_inventory_item(
        &inv_pool,
        &tenant,
        "SKU-001",
        "Bolt M6",
        Some("Stainless M6 bolt"),
    )
    .await;
    insert_standard_cost(&inv_pool, &tenant, item_id, 150).await;

    // Create BOM with that item as a component
    let bom = bom_service::create_bom(
        &bom_pool,
        &tenant,
        &CreateBomRequest {
            part_id: Uuid::new_v4(),
            description: None,
        },
        &corr,
        None,
    )
    .await
    .expect("create_bom");

    let rev = bom_service::create_revision(
        &bom_pool,
        &tenant,
        bom.id,
        &CreateRevisionRequest {
            revision_label: "Rev-B".to_string(),
        },
        &corr,
        None,
    )
    .await
    .expect("create_revision");

    bom_service::add_line(
        &bom_pool,
        &tenant,
        rev.id,
        &AddLineRequest {
            component_item_id: item_id,
            quantity: 4.0,
            uom: Some("EA".to_string()),
            scrap_factor: None,
            find_number: Some(10),
        },
        &corr,
        None,
    )
    .await
    .expect("add_line");

    let inventory = InventoryClient::direct(inv_pool);
    let enriched = list_lines_enriched(&bom_pool, &tenant, rev.id, &inventory, &claims)
        .await
        .expect("list_lines_enriched");

    assert_eq!(enriched.len(), 1, "Expected 1 enriched line");
    let item = enriched[0].item.as_ref().expect("item should be populated");
    assert_eq!(item.item_id, item_id);
    assert_eq!(item.sku, "SKU-001");
    assert_eq!(item.name, "Bolt M6");
    assert_eq!(item.description.as_deref(), Some("Stainless M6 bolt"));
    assert_eq!(
        item.unit_cost_minor,
        Some(150),
        "standard cost should be 150 minor units"
    );
}

/// An unresolvable `component_item_id` (no matching inventory item) must produce
/// `item: null`, never a 500 or error.
#[tokio::test]
#[serial]
async fn unresolvable_part_id_returns_null_item() {
    let bom_pool = setup_bom_db().await;
    let inv_pool = setup_inventory_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();
    let claims = dummy_claims();
    let ghost_id = Uuid::new_v4(); // Does not exist in inventory

    let bom = bom_service::create_bom(
        &bom_pool,
        &tenant,
        &CreateBomRequest {
            part_id: Uuid::new_v4(),
            description: None,
        },
        &corr,
        None,
    )
    .await
    .expect("create_bom");

    let rev = bom_service::create_revision(
        &bom_pool,
        &tenant,
        bom.id,
        &CreateRevisionRequest {
            revision_label: "Rev-C".to_string(),
        },
        &corr,
        None,
    )
    .await
    .expect("create_revision");

    bom_service::add_line(
        &bom_pool,
        &tenant,
        rev.id,
        &AddLineRequest {
            component_item_id: ghost_id,
            quantity: 1.0,
            uom: None,
            scrap_factor: None,
            find_number: None,
        },
        &corr,
        None,
    )
    .await
    .expect("add_line with ghost id");

    let inventory = InventoryClient::direct(inv_pool);
    let enriched = list_lines_enriched(&bom_pool, &tenant, rev.id, &inventory, &claims)
        .await
        .expect("list_lines_enriched should not error on unresolvable id");

    assert_eq!(enriched.len(), 1);
    assert!(
        enriched[0].item.is_none(),
        "item should be null for unresolvable part_id"
    );
}

/// Item with no standard cost config produces `unit_cost_minor: null`.
#[tokio::test]
#[serial]
async fn item_without_standard_cost_has_null_unit_cost() {
    let bom_pool = setup_bom_db().await;
    let inv_pool = setup_inventory_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();
    let claims = dummy_claims();

    // Create item without valuation config
    let item_id = insert_inventory_item(&inv_pool, &tenant, "SKU-NO-COST", "Widget", None).await;

    let bom = bom_service::create_bom(
        &bom_pool,
        &tenant,
        &CreateBomRequest {
            part_id: Uuid::new_v4(),
            description: None,
        },
        &corr,
        None,
    )
    .await
    .expect("create_bom");

    let rev = bom_service::create_revision(
        &bom_pool,
        &tenant,
        bom.id,
        &CreateRevisionRequest {
            revision_label: "Rev-D".to_string(),
        },
        &corr,
        None,
    )
    .await
    .expect("create_revision");

    bom_service::add_line(
        &bom_pool,
        &tenant,
        rev.id,
        &AddLineRequest {
            component_item_id: item_id,
            quantity: 3.0,
            uom: None,
            scrap_factor: None,
            find_number: None,
        },
        &corr,
        None,
    )
    .await
    .expect("add_line");

    let inventory = InventoryClient::direct(inv_pool);
    let enriched = list_lines_enriched(&bom_pool, &tenant, rev.id, &inventory, &claims)
        .await
        .expect("list_lines_enriched");

    let item = enriched[0].item.as_ref().expect("item should be populated");
    assert_eq!(item.sku, "SKU-NO-COST");
    assert!(
        item.unit_cost_minor.is_none(),
        "unit_cost_minor should be null when no standard cost config exists"
    );
}
