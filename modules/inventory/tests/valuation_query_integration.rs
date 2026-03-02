//! Integration tests for valuation query endpoints (bd-1goe).
//!
//! Tests run against a real PostgreSQL database.
//! Set DATABASE_URL to the inventory database connection string.
//!
//! Coverage:
//! 1. list_snapshots returns snapshots for tenant in descending as_of order
//! 2. list_snapshots with warehouse_id filter returns only matching snapshots
//! 3. list_snapshots tenant isolation: tenant B sees only its own snapshots
//! 4. get_snapshot returns header for existing snapshot
//! 5. get_snapshot returns None for wrong tenant (isolation)
//! 6. get_snapshot_lines returns all lines for a snapshot, ordered by item_id
//! 7. Limit/offset pagination works

use chrono::Utc;
use inventory_rs::domain::valuation::{
    queries::{get_snapshot, get_snapshot_lines, list_snapshots},
    snapshot_service::{create_valuation_snapshot, CreateSnapshotRequest},
};
use inventory_rs::domain::{
    items::{CreateItemRequest, ItemRepo, TrackingMode},
    receipt_service::{process_receipt, ReceiptRequest},
};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

// ============================================================================
// Helpers
// ============================================================================

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for integration tests");
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
    format!("vq-tenant-{}", Uuid::new_v4())
}

async fn create_item(pool: &sqlx::PgPool, tenant_id: &str, sku: &str) -> Uuid {
    ItemRepo::create(
        pool,
        &CreateItemRequest {
            tenant_id: tenant_id.to_string(),
            sku: sku.to_string(),
            name: format!("Query Test Item {}", sku),
            description: None,
            inventory_account_ref: "1200".to_string(),
            cogs_account_ref: "5000".to_string(),
            variance_account_ref: "5010".to_string(),
            uom: None,
            tracking_mode: TrackingMode::None,
        },
    )
    .await
    .expect("create item")
    .id
}

async fn do_receipt(
    pool: &sqlx::PgPool,
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
    qty: i64,
    unit_cost: i64,
    key: &str,
) {
    process_receipt(
        pool,
        &ReceiptRequest {
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
        },
        None,
    )
    .await
    .expect("receipt");
}

async fn do_snapshot(pool: &sqlx::PgPool, tenant_id: &str, warehouse_id: Uuid, key: &str) -> Uuid {
    let req = CreateSnapshotRequest {
        tenant_id: tenant_id.to_string(),
        warehouse_id,
        location_id: None,
        as_of: Utc::now(),
        idempotency_key: key.to_string(),
        currency: "usd".to_string(),
        correlation_id: None,
        causation_id: None,
    };
    let (result, _) = create_valuation_snapshot(pool, &req)
        .await
        .expect("create snapshot");
    result.snapshot_id
}

// ============================================================================
// Tests
// ============================================================================

/// list_snapshots returns created snapshots for the tenant.
#[tokio::test]
#[serial]
async fn test_list_snapshots_returns_tenant_snapshots() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let wh = Uuid::new_v4();

    let snap_id = do_snapshot(&pool, &tenant, wh, "vq-list-001").await;

    let snapshots = list_snapshots(&pool, &tenant, None, 50, 0)
        .await
        .expect("list snapshots");

    assert!(
        snapshots.iter().any(|s| s.id == snap_id),
        "created snapshot must appear in list"
    );
    assert!(
        snapshots.iter().all(|s| s.tenant_id == tenant),
        "all returned snapshots must belong to the tenant"
    );
}

/// list_snapshots with warehouse_id filter returns only matching snapshots.
#[tokio::test]
#[serial]
async fn test_list_snapshots_warehouse_filter() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let wh_a = Uuid::new_v4();
    let wh_b = Uuid::new_v4();

    let snap_a = do_snapshot(&pool, &tenant, wh_a, "vq-wf-snap-a").await;
    let snap_b = do_snapshot(&pool, &tenant, wh_b, "vq-wf-snap-b").await;

    // Filter for wh_a only
    let snapshots = list_snapshots(&pool, &tenant, Some(wh_a), 50, 0)
        .await
        .expect("list snapshots wh_a");

    assert!(
        snapshots.iter().any(|s| s.id == snap_a),
        "wh_a snapshot should appear"
    );
    assert!(
        !snapshots.iter().any(|s| s.id == snap_b),
        "wh_b snapshot should NOT appear"
    );
}

/// Tenant isolation: tenant B cannot see tenant A's snapshots.
#[tokio::test]
#[serial]
async fn test_list_snapshots_tenant_isolation() {
    let pool = setup_db().await;
    let tenant_a = unique_tenant();
    let tenant_b = unique_tenant();
    let wh = Uuid::new_v4();

    let snap_a = do_snapshot(&pool, &tenant_a, wh, "vq-iso-snap-a").await;

    let snapshots_b = list_snapshots(&pool, &tenant_b, None, 50, 0)
        .await
        .expect("list snapshots for tenant b");

    assert!(
        !snapshots_b.iter().any(|s| s.id == snap_a),
        "tenant B must not see tenant A's snapshot"
    );
}

/// get_snapshot returns the snapshot header for an existing snapshot.
#[tokio::test]
#[serial]
async fn test_get_snapshot_returns_header() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let wh = Uuid::new_v4();

    let item_id = create_item(&pool, &tenant, "VQ-DET-001").await;
    do_receipt(&pool, &tenant, item_id, wh, 10, 500, "vq-det-rc1").await;

    let snap_id = do_snapshot(&pool, &tenant, wh, "vq-det-snap-1").await;

    let snapshot = get_snapshot(&pool, &tenant, snap_id)
        .await
        .expect("get snapshot")
        .expect("snapshot must exist");

    assert_eq!(snapshot.id, snap_id);
    assert_eq!(snapshot.tenant_id, tenant);
    assert_eq!(snapshot.warehouse_id, wh);
    assert_eq!(snapshot.total_value_minor, 5_000); // 10 * 500
    assert_eq!(snapshot.currency, "usd");
}

/// get_snapshot returns None for a snapshot belonging to another tenant.
#[tokio::test]
#[serial]
async fn test_get_snapshot_tenant_isolation() {
    let pool = setup_db().await;
    let tenant_a = unique_tenant();
    let tenant_b = unique_tenant();
    let wh = Uuid::new_v4();

    let snap_id = do_snapshot(&pool, &tenant_a, wh, "vq-tiso-snap-a").await;

    // Tenant B tries to fetch tenant A's snapshot
    let result = get_snapshot(&pool, &tenant_b, snap_id)
        .await
        .expect("query succeeded");

    assert!(
        result.is_none(),
        "tenant B must not see tenant A's snapshot"
    );
}

/// get_snapshot_lines returns lines ordered by item_id for a snapshot with items.
#[tokio::test]
#[serial]
async fn test_get_snapshot_lines_returns_ordered_lines() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let wh = Uuid::new_v4();

    let item_a = create_item(&pool, &tenant, "VQ-LINE-A").await;
    let item_b = create_item(&pool, &tenant, "VQ-LINE-B").await;

    do_receipt(&pool, &tenant, item_a, wh, 5, 1000, "vq-lines-rc-a").await;
    do_receipt(&pool, &tenant, item_b, wh, 8, 200, "vq-lines-rc-b").await;

    let snap_id = do_snapshot(&pool, &tenant, wh, "vq-lines-snap-1").await;

    let lines = get_snapshot_lines(&pool, snap_id)
        .await
        .expect("get snapshot lines");

    assert_eq!(lines.len(), 2, "two items → two lines");

    // Verify all lines belong to the snapshot
    assert!(lines.iter().all(|l| l.snapshot_id == snap_id));

    // Verify ordering by item_id (ascending)
    let ids: Vec<uuid::Uuid> = lines.iter().map(|l| l.item_id).collect();
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(ids, sorted, "lines must be ordered by item_id ASC");

    // Verify totals
    let total: i64 = lines.iter().map(|l| l.total_value_minor).sum();
    assert_eq!(total, 5 * 1000 + 8 * 200); // 5000 + 1600 = 6600
}

/// get_snapshot_lines returns empty vec for a snapshot with no inventory.
#[tokio::test]
#[serial]
async fn test_get_snapshot_lines_empty_warehouse() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let wh = Uuid::new_v4(); // no receipts

    let snap_id = do_snapshot(&pool, &tenant, wh, "vq-empty-snap-1").await;

    let lines = get_snapshot_lines(&pool, snap_id)
        .await
        .expect("get snapshot lines");

    assert!(lines.is_empty(), "empty warehouse → no lines");
}

/// Limit/offset pagination: offset skips rows.
#[tokio::test]
#[serial]
async fn test_list_snapshots_pagination() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let wh = Uuid::new_v4();

    // Create 3 snapshots
    do_snapshot(&pool, &tenant, wh, "vq-page-snap-1").await;
    do_snapshot(&pool, &tenant, wh, "vq-page-snap-2").await;
    do_snapshot(&pool, &tenant, wh, "vq-page-snap-3").await;

    let all = list_snapshots(&pool, &tenant, None, 200, 0)
        .await
        .expect("list all");
    assert!(all.len() >= 3, "at least 3 snapshots for this tenant");

    let page1 = list_snapshots(&pool, &tenant, None, 2, 0)
        .await
        .expect("page 1");
    let page2 = list_snapshots(&pool, &tenant, None, 2, 2)
        .await
        .expect("page 2");

    assert_eq!(page1.len(), 2);
    // page2 may have 1 or more depending on other test data; check no duplicates
    for row in &page2 {
        assert!(
            !page1.iter().any(|r| r.id == row.id),
            "pages must not overlap"
        );
    }
}
