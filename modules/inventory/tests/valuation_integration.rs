//! Integration tests for valuation snapshot builder (bd-2k0i).
//!
//! Tests run against a real PostgreSQL database.
//! Set DATABASE_URL to the inventory database connection string.
//!
//! Coverage:
//! 1. POST snapshot creates header + lines deterministically from FIFO layers
//! 2. Idempotency: same key + same body replays stored result (HTTP 200)
//! 3. Idempotency key conflict: same key + different body returns error
//! 4. Outbox contains inventory.valuation_snapshot_created with correct metadata
//! 5. Empty warehouse produces a zero-value snapshot (no items, total = 0)

use chrono::Utc;
use inventory_rs::domain::{
    items::{CreateItemRequest, ItemRepo},
    receipt_service::{process_receipt, ReceiptRequest},
    valuation::snapshot_service::{
        create_valuation_snapshot, CreateSnapshotRequest, SnapshotError,
    },
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

async fn create_item(pool: &sqlx::PgPool, tenant_id: &str, sku: &str) -> Uuid {
    ItemRepo::create(
        pool,
        &CreateItemRequest {
            tenant_id: tenant_id.to_string(),
            sku: sku.to_string(),
            name: "Valuation Test Item".to_string(),
            description: None,
            inventory_account_ref: "1200".to_string(),
            cogs_account_ref: "5000".to_string(),
            variance_account_ref: "5010".to_string(),
            uom: None,
            tracking_mode: inventory_rs::domain::items::TrackingMode::None,
        },
    )
    .await
    .expect("create item")
    .id
}

async fn cleanup(pool: &sqlx::PgPool, tenant_id: &str) {
    sqlx::query("DELETE FROM inv_outbox WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM inv_idempotency_keys WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM inventory_valuation_lines WHERE snapshot_id IN (SELECT id FROM inventory_valuation_snapshots WHERE tenant_id = $1)")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM inventory_valuation_snapshots WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM layer_consumptions WHERE EXISTS (SELECT 1 FROM inventory_layers il WHERE il.id = layer_id AND il.tenant_id = $1)")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM inventory_layers WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM inventory_ledger WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM item_on_hand WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM items WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
}

fn make_snapshot_req(tenant_id: &str, warehouse_id: Uuid, idem: &str) -> CreateSnapshotRequest {
    CreateSnapshotRequest {
        tenant_id: tenant_id.to_string(),
        warehouse_id,
        location_id: None,
        as_of: Utc::now(),
        idempotency_key: idem.to_string(),
        currency: "usd".to_string(),
        correlation_id: Some("corr-val-test".to_string()),
        causation_id: None,
    }
}

// ============================================================================
// Tests
// ============================================================================

#[tokio::test]
#[serial]
async fn test_snapshot_creates_header_and_lines() {
    let pool = setup_db().await;
    let tenant = format!("t-val-basic-{}", Uuid::new_v4());
    let wh = Uuid::new_v4();
    cleanup(&pool, &tenant).await;

    let item_id = create_item(&pool, &tenant, &format!("VAL-{}", Uuid::new_v4())).await;

    // Receive 10 units @ $5.00 each
    process_receipt(
        &pool,
        &ReceiptRequest {
            tenant_id: tenant.clone(),
            item_id,
            warehouse_id: wh,
            quantity: 10,
            unit_cost_minor: 500,
            currency: "usd".to_string(),
            source_type: "purchase".to_string(),
            purchase_order_id: None,
            idempotency_key: "rc-val-basic".to_string(),
            correlation_id: None,
            causation_id: None,
            lot_code: None,
            serial_codes: None,
            location_id: None,
            uom_id: None,
        },
        None,
    )
    .await
    .expect("receipt");

    let req = make_snapshot_req(&tenant, wh, "snap-basic-1");
    let (result, is_replay) = create_valuation_snapshot(&pool, &req)
        .await
        .expect("create snapshot");

    assert!(!is_replay, "first call is not a replay");
    assert_eq!(result.tenant_id, tenant);
    assert_eq!(result.warehouse_id, wh);
    assert_eq!(result.line_count, 1, "one item = one line");
    assert_eq!(result.total_value_minor, 5_000, "10 * 500 = 5000");
    assert_eq!(result.lines.len(), 1);
    assert_eq!(result.lines[0].item_id, item_id);
    assert_eq!(result.lines[0].quantity_on_hand, 10);
    assert_eq!(result.lines[0].unit_cost_minor, 500);
    assert_eq!(result.lines[0].total_value_minor, 5_000);

    cleanup(&pool, &tenant).await;
}

#[tokio::test]
#[serial]
async fn test_idempotency_replay_returns_stored_result() {
    let pool = setup_db().await;
    let tenant = format!("t-val-idem-{}", Uuid::new_v4());
    let wh = Uuid::new_v4();
    cleanup(&pool, &tenant).await;

    let item_id = create_item(&pool, &tenant, &format!("VAL-IDEM-{}", Uuid::new_v4())).await;

    process_receipt(
        &pool,
        &ReceiptRequest {
            tenant_id: tenant.clone(),
            item_id,
            warehouse_id: wh,
            quantity: 5,
            unit_cost_minor: 1000,
            currency: "usd".to_string(),
            source_type: "purchase".to_string(),
            purchase_order_id: None,
            idempotency_key: "rc-val-idem".to_string(),
            correlation_id: None,
            causation_id: None,
            lot_code: None,
            serial_codes: None,
            location_id: None,
            uom_id: None,
        },
        None,
    )
    .await
    .expect("receipt");

    let req = make_snapshot_req(&tenant, wh, "snap-idem-key");

    let (first, is_replay_first) = create_valuation_snapshot(&pool, &req)
        .await
        .expect("first snapshot");
    assert!(!is_replay_first);

    let (second, is_replay_second) = create_valuation_snapshot(&pool, &req)
        .await
        .expect("idempotent replay");
    assert!(is_replay_second, "second call with same key is a replay");
    assert_eq!(
        first.snapshot_id, second.snapshot_id,
        "same snapshot_id returned"
    );
    assert_eq!(first.total_value_minor, second.total_value_minor);

    cleanup(&pool, &tenant).await;
}

#[tokio::test]
#[serial]
async fn test_idempotency_conflict_returns_error() {
    let pool = setup_db().await;
    let tenant = format!("t-val-conflict-{}", Uuid::new_v4());
    let wh_a = Uuid::new_v4();
    let wh_b = Uuid::new_v4();
    cleanup(&pool, &tenant).await;

    let req_a = make_snapshot_req(&tenant, wh_a, "snap-conflict-key");
    create_valuation_snapshot(&pool, &req_a)
        .await
        .expect("first snapshot");

    // Same idempotency key, different warehouse_id → conflict
    let req_b = make_snapshot_req(&tenant, wh_b, "snap-conflict-key");
    let err = create_valuation_snapshot(&pool, &req_b)
        .await
        .expect_err("should be a conflict");

    assert!(
        matches!(err, SnapshotError::ConflictingIdempotencyKey),
        "expected ConflictingIdempotencyKey, got {:?}",
        err
    );

    cleanup(&pool, &tenant).await;
}

#[tokio::test]
#[serial]
async fn test_outbox_contains_snapshot_event() {
    let pool = setup_db().await;
    let tenant = format!("t-val-outbox-{}", Uuid::new_v4());
    let wh = Uuid::new_v4();
    cleanup(&pool, &tenant).await;

    let item_id = create_item(&pool, &tenant, &format!("VAL-OUT-{}", Uuid::new_v4())).await;

    process_receipt(
        &pool,
        &ReceiptRequest {
            tenant_id: tenant.clone(),
            item_id,
            warehouse_id: wh,
            quantity: 8,
            unit_cost_minor: 250,
            currency: "usd".to_string(),
            source_type: "purchase".to_string(),
            purchase_order_id: None,
            idempotency_key: "rc-val-outbox".to_string(),
            correlation_id: None,
            causation_id: None,
            lot_code: None,
            serial_codes: None,
            location_id: None,
            uom_id: None,
        },
        None,
    )
    .await
    .expect("receipt");

    let req = make_snapshot_req(&tenant, wh, "snap-outbox-key");
    let (result, _) = create_valuation_snapshot(&pool, &req)
        .await
        .expect("snapshot");

    // Verify outbox row exists with correct event_type and tenant
    let row: Option<(String, String)> = sqlx::query_as(
        r#"
        SELECT event_type, tenant_id
        FROM inv_outbox
        WHERE tenant_id = $1
          AND aggregate_type = 'valuation_snapshot'
          AND aggregate_id = $2
        "#,
    )
    .bind(&tenant)
    .bind(result.snapshot_id.to_string())
    .fetch_optional(&pool)
    .await
    .expect("outbox query");

    let (event_type, outbox_tenant) = row.expect("outbox row must exist");
    assert_eq!(event_type, "inventory.valuation_snapshot_created");
    assert_eq!(outbox_tenant, tenant);

    cleanup(&pool, &tenant).await;
}

#[tokio::test]
#[serial]
async fn test_empty_warehouse_snapshot_has_zero_total() {
    let pool = setup_db().await;
    let tenant = format!("t-val-empty-{}", Uuid::new_v4());
    let wh = Uuid::new_v4(); // empty warehouse — no receipts
    cleanup(&pool, &tenant).await;

    let req = make_snapshot_req(&tenant, wh, "snap-empty-wh");
    let (result, is_replay) = create_valuation_snapshot(&pool, &req)
        .await
        .expect("empty snapshot");

    assert!(!is_replay);
    assert_eq!(
        result.total_value_minor, 0,
        "empty warehouse has zero value"
    );
    assert_eq!(result.line_count, 0, "no items = no lines");
    assert!(result.lines.is_empty());

    cleanup(&pool, &tenant).await;
}
