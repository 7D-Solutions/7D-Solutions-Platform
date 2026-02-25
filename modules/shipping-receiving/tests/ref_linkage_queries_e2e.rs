//! E2E integration tests for AP/AR ref linkage query endpoints (bd-1zr5).
//!
//! Tests run against a real PostgreSQL database. Proves:
//! 1. Inbound lines with PO refs can be queried by po_id and po_line_id
//! 2. Outbound lines with source_ref can be queried by ref_type + ref_id
//! 3. All queries are tenant-isolated
//! 4. Empty results returned for non-matching refs

use serial_test::serial;
use shipping_receiving_rs::db::repository::ShipmentRepository;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

// ── Test DB helpers ─────────────────────────────────────────

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for integration tests");

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to shipping-receiving test DB");

    let _ = sqlx::migrate!("db/migrations").run(&pool).await;

    pool
}

/// Insert a shipment directly via SQL and return its id.
async fn insert_shipment(pool: &sqlx::PgPool, tenant_id: Uuid, direction: &str) -> Uuid {
    let row: (Uuid,) = sqlx::query_as(
        r#"
        INSERT INTO shipments (tenant_id, direction, status)
        VALUES ($1, $2, 'draft')
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(direction)
    .fetch_one(pool)
    .await
    .expect("insert shipment");
    row.0
}

/// Insert a shipment line with PO refs and return its id.
async fn insert_line_with_po(
    pool: &sqlx::PgPool,
    tenant_id: Uuid,
    shipment_id: Uuid,
    po_id: Uuid,
    po_line_id: Uuid,
) -> Uuid {
    let row: (Uuid,) = sqlx::query_as(
        r#"
        INSERT INTO shipment_lines
            (tenant_id, shipment_id, sku, qty_expected, po_id, po_line_id)
        VALUES ($1, $2, 'TEST-SKU', 10, $3, $4)
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(shipment_id)
    .bind(po_id)
    .bind(po_line_id)
    .fetch_one(pool)
    .await
    .expect("insert line with po");
    row.0
}

/// Insert a shipment line with source_ref and return its id.
async fn insert_line_with_source_ref(
    pool: &sqlx::PgPool,
    tenant_id: Uuid,
    shipment_id: Uuid,
    ref_type: &str,
    ref_id: Uuid,
) -> Uuid {
    let row: (Uuid,) = sqlx::query_as(
        r#"
        INSERT INTO shipment_lines
            (tenant_id, shipment_id, sku, qty_expected, source_ref_type, source_ref_id)
        VALUES ($1, $2, 'TEST-SKU', 5, $3, $4)
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(shipment_id)
    .bind(ref_type)
    .bind(ref_id)
    .fetch_one(pool)
    .await
    .expect("insert line with source_ref");
    row.0
}

// ── Tests ───────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn find_shipments_by_po_returns_matching_shipments() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let po_id = Uuid::new_v4();
    let po_line_id = Uuid::new_v4();

    // Create two inbound shipments, both referencing the same PO
    let ship1 = insert_shipment(&pool, tenant_id, "inbound").await;
    let ship2 = insert_shipment(&pool, tenant_id, "inbound").await;

    insert_line_with_po(&pool, tenant_id, ship1, po_id, po_line_id).await;
    insert_line_with_po(&pool, tenant_id, ship2, po_id, Uuid::new_v4()).await;

    // Create a third shipment with a different PO — should not appear
    let ship3 = insert_shipment(&pool, tenant_id, "inbound").await;
    insert_line_with_po(&pool, tenant_id, ship3, Uuid::new_v4(), Uuid::new_v4()).await;

    let results = ShipmentRepository::find_shipments_by_po(&pool, tenant_id, po_id)
        .await
        .expect("query by po_id");

    let ids: Vec<Uuid> = results.iter().map(|s| s.id).collect();
    assert!(ids.contains(&ship1), "ship1 must be in results");
    assert!(ids.contains(&ship2), "ship2 must be in results");
    assert!(!ids.contains(&ship3), "ship3 must NOT be in results");
}

#[tokio::test]
#[serial]
async fn find_lines_by_po_line_returns_matching_lines() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let po_id = Uuid::new_v4();
    let po_line_id = Uuid::new_v4();

    let ship1 = insert_shipment(&pool, tenant_id, "inbound").await;
    let line1 = insert_line_with_po(&pool, tenant_id, ship1, po_id, po_line_id).await;

    // Second line with a different po_line_id — should not appear
    let _line2 = insert_line_with_po(&pool, tenant_id, ship1, po_id, Uuid::new_v4()).await;

    let results = ShipmentRepository::find_lines_by_po_line(&pool, tenant_id, po_line_id)
        .await
        .expect("query by po_line_id");

    assert_eq!(results.len(), 1, "exactly one line must match");
    assert_eq!(results[0].id, line1);
    assert_eq!(results[0].po_id, Some(po_id));
    assert_eq!(results[0].po_line_id, Some(po_line_id));
}

#[tokio::test]
#[serial]
async fn find_shipments_by_source_ref_returns_matching_shipments() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let so_id = Uuid::new_v4();

    // Create outbound shipment with a sales_order source ref
    let ship1 = insert_shipment(&pool, tenant_id, "outbound").await;
    insert_line_with_source_ref(&pool, tenant_id, ship1, "sales_order", so_id).await;

    // Create another shipment with a different ref — should not appear
    let ship2 = insert_shipment(&pool, tenant_id, "outbound").await;
    insert_line_with_source_ref(&pool, tenant_id, ship2, "transfer_request", Uuid::new_v4())
        .await;

    let results =
        ShipmentRepository::find_by_source_ref(&pool, tenant_id, "sales_order", so_id)
            .await
            .expect("query by source_ref");

    let ids: Vec<Uuid> = results.iter().map(|s| s.id).collect();
    assert_eq!(ids.len(), 1, "exactly one shipment must match");
    assert!(ids.contains(&ship1), "ship1 must be in results");
}

#[tokio::test]
#[serial]
async fn ref_queries_are_tenant_isolated() {
    let pool = setup_db().await;
    let tenant_a = Uuid::new_v4();
    let tenant_b = Uuid::new_v4();
    let po_id = Uuid::new_v4();
    let so_id = Uuid::new_v4();

    // Tenant A: inbound with PO ref
    let ship_a = insert_shipment(&pool, tenant_a, "inbound").await;
    insert_line_with_po(&pool, tenant_a, ship_a, po_id, Uuid::new_v4()).await;

    // Tenant A: outbound with source ref
    let ship_a2 = insert_shipment(&pool, tenant_a, "outbound").await;
    insert_line_with_source_ref(&pool, tenant_a, ship_a2, "sales_order", so_id).await;

    // Tenant B queries the same po_id and source_ref — must get empty results
    let po_results = ShipmentRepository::find_shipments_by_po(&pool, tenant_b, po_id)
        .await
        .expect("tenant B po query");
    assert!(
        po_results.is_empty(),
        "tenant B must not see tenant A's PO shipments"
    );

    let src_results =
        ShipmentRepository::find_by_source_ref(&pool, tenant_b, "sales_order", so_id)
            .await
            .expect("tenant B source_ref query");
    assert!(
        src_results.is_empty(),
        "tenant B must not see tenant A's source_ref shipments"
    );
}

#[tokio::test]
#[serial]
async fn empty_results_for_nonexistent_refs() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();

    let po_results =
        ShipmentRepository::find_shipments_by_po(&pool, tenant_id, Uuid::new_v4())
            .await
            .expect("nonexistent po_id");
    assert!(po_results.is_empty());

    let line_results =
        ShipmentRepository::find_lines_by_po_line(&pool, tenant_id, Uuid::new_v4())
            .await
            .expect("nonexistent po_line_id");
    assert!(line_results.is_empty());

    let src_results = ShipmentRepository::find_by_source_ref(
        &pool,
        tenant_id,
        "sales_order",
        Uuid::new_v4(),
    )
    .await
    .expect("nonexistent source_ref");
    assert!(src_results.is_empty());
}
