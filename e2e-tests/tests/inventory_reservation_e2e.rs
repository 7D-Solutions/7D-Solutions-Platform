//! E2E Test: Inventory Reservation Flow (bd-2y1a)
//!
//! ## Coverage
//! 1. inventory_reservation_reserve_creates_atomically  — row + projection + outbox in one tx
//! 2. inventory_reservation_release_compensates         — compensating row references original
//! 3. inventory_reservation_idempotency_reserve         — duplicate key returns stored result
//! 4. inventory_reservation_idempotency_release         — duplicate release key is idempotent
//! 5. inventory_reservation_double_release_rejected     — AlreadyReleased guard
//! 6. inventory_reservation_available_quantity          — available = on_hand - reserved
//!
//! ## Setup
//! Set INVENTORY_DATABASE_URL (or DATABASE_URL) to the inventory DB connection string.

use inventory_rs::domain::{
    items::{CreateItemRequest, ItemRepo},
    reservation_service::{
        process_release, process_reserve, ReleaseRequest, ReservationError, ReserveRequest,
    },
};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

// ============================================================================
// Helpers
// ============================================================================

async fn get_inventory_pool() -> sqlx::PgPool {
    let url = std::env::var("INVENTORY_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| {
            "postgresql://inventory_user:inventory_pass@localhost:5442/inventory_db".to_string()
        });

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to inventory DB — is INVENTORY_DATABASE_URL set?");

    sqlx::migrate!("../modules/inventory/db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run inventory migrations");

    pool
}

fn test_item_req(tenant_id: &str, sku: &str) -> CreateItemRequest {
    CreateItemRequest {
        tenant_id: tenant_id.to_string(),
        sku: sku.to_string(),
        name: "E2E Reservation Item".to_string(),
        description: None,
        inventory_account_ref: "1200".to_string(),
        cogs_account_ref: "5000".to_string(),
        variance_account_ref: "5010".to_string(),
        uom: None,
    }
}

fn test_reserve_req(tenant_id: &str, item_id: Uuid, warehouse_id: Uuid) -> ReserveRequest {
    ReserveRequest {
        tenant_id: tenant_id.to_string(),
        item_id,
        warehouse_id,
        quantity: 50,
        reference_type: Some("sales_order".to_string()),
        reference_id: Some(format!("SO-E2E-{}", Uuid::new_v4())),
        expires_at: None,
        idempotency_key: format!("e2e-res-{}", Uuid::new_v4()),
        correlation_id: Some("e2e-corr".to_string()),
        causation_id: None,
    }
}

async fn cleanup_tenant(pool: &sqlx::PgPool, tenant_id: &str) {
    sqlx::query("DELETE FROM inv_outbox WHERE tenant_id = $1").bind(tenant_id).execute(pool).await.ok();
    sqlx::query("DELETE FROM inv_idempotency_keys WHERE tenant_id = $1").bind(tenant_id).execute(pool).await.ok();
    sqlx::query("DELETE FROM inventory_reservations WHERE tenant_id = $1").bind(tenant_id).execute(pool).await.ok();
    sqlx::query("DELETE FROM item_on_hand WHERE tenant_id = $1").bind(tenant_id).execute(pool).await.ok();
    sqlx::query("DELETE FROM inventory_layers WHERE tenant_id = $1").bind(tenant_id).execute(pool).await.ok();
    sqlx::query("DELETE FROM inventory_ledger WHERE tenant_id = $1").bind(tenant_id).execute(pool).await.ok();
    sqlx::query("DELETE FROM items WHERE tenant_id = $1").bind(tenant_id).execute(pool).await.ok();
}

// ============================================================================
// Tests
// ============================================================================

#[tokio::test]
#[serial]
async fn inventory_reservation_reserve_creates_atomically() {
    let pool = get_inventory_pool().await;
    let tenant_id = format!("e2e-{}", Uuid::new_v4());
    let warehouse_id = Uuid::new_v4();

    let item = ItemRepo::create(&pool, &test_item_req(&tenant_id, "E2E-RSV-001"))
        .await
        .expect("create item");

    let req = test_reserve_req(&tenant_id, item.id, warehouse_id);
    let (result, is_replay) = process_reserve(&pool, &req)
        .await
        .expect("reserve must succeed");

    assert!(!is_replay, "must be a new reservation");
    assert_eq!(result.quantity, 50);

    // Reservation row: status=active, no compensating linkage.
    let (status, reverses): (String, Option<Uuid>) = sqlx::query_as(
        "SELECT status::TEXT, reverses_reservation_id FROM inventory_reservations WHERE id = $1",
    )
    .bind(result.reservation_id)
    .fetch_one(&pool)
    .await
    .expect("reservation row");

    assert_eq!(status, "active");
    assert!(reverses.is_none());

    // On-hand projection: quantity_reserved updated.
    let qty_reserved: i64 = sqlx::query_scalar(
        "SELECT quantity_reserved FROM item_on_hand WHERE tenant_id = $1 AND item_id = $2 AND warehouse_id = $3",
    )
    .bind(&tenant_id)
    .bind(item.id)
    .bind(warehouse_id)
    .fetch_one(&pool)
    .await
    .expect("on-hand row");

    assert_eq!(qty_reserved, 50);

    // Outbox event: correct type.
    let event_type: String = sqlx::query_scalar(
        "SELECT event_type FROM inv_outbox WHERE event_id = $1",
    )
    .bind(result.event_id)
    .fetch_one(&pool)
    .await
    .expect("outbox event");

    assert_eq!(event_type, "inventory.item_reserved");

    cleanup_tenant(&pool, &tenant_id).await;
}

#[tokio::test]
#[serial]
async fn inventory_reservation_release_compensates() {
    let pool = get_inventory_pool().await;
    let tenant_id = format!("e2e-{}", Uuid::new_v4());
    let warehouse_id = Uuid::new_v4();

    let item = ItemRepo::create(&pool, &test_item_req(&tenant_id, "E2E-REL-001"))
        .await
        .expect("create item");

    let (reserve_result, _) = process_reserve(&pool, &test_reserve_req(&tenant_id, item.id, warehouse_id))
        .await
        .expect("reserve");

    let release_req = ReleaseRequest {
        tenant_id: tenant_id.clone(),
        reservation_id: reserve_result.reservation_id,
        idempotency_key: format!("e2e-rel-{}", Uuid::new_v4()),
        correlation_id: None,
        causation_id: None,
    };
    let (release_result, is_replay) = process_release(&pool, &release_req)
        .await
        .expect("release must succeed");

    assert!(!is_replay);
    assert_eq!(release_result.reservation_id, reserve_result.reservation_id);

    // Compensating row: status=released, points to original.
    let (status, reverses): (String, Option<Uuid>) = sqlx::query_as(
        "SELECT status::TEXT, reverses_reservation_id FROM inventory_reservations WHERE id = $1",
    )
    .bind(release_result.release_id)
    .fetch_one(&pool)
    .await
    .expect("release row");

    assert_eq!(status, "released");
    assert_eq!(reverses, Some(reserve_result.reservation_id),
        "compensating row must reference original reserve");

    // Projection: quantity_reserved back to 0.
    let qty_reserved: i64 = sqlx::query_scalar(
        "SELECT quantity_reserved FROM item_on_hand WHERE tenant_id = $1 AND item_id = $2 AND warehouse_id = $3",
    )
    .bind(&tenant_id)
    .bind(item.id)
    .bind(warehouse_id)
    .fetch_one(&pool)
    .await
    .expect("on-hand row");

    assert_eq!(qty_reserved, 0, "quantity_reserved must be 0 after release");

    cleanup_tenant(&pool, &tenant_id).await;
}

#[tokio::test]
#[serial]
async fn inventory_reservation_idempotency_reserve() {
    let pool = get_inventory_pool().await;
    let tenant_id = format!("e2e-{}", Uuid::new_v4());
    let warehouse_id = Uuid::new_v4();

    let item = ItemRepo::create(&pool, &test_item_req(&tenant_id, "E2E-IDEM-RSV-001"))
        .await
        .expect("create item");

    let req = test_reserve_req(&tenant_id, item.id, warehouse_id);
    let (r1, _) = process_reserve(&pool, &req).await.expect("first reserve");
    let (r2, is_replay) = process_reserve(&pool, &req).await.expect("second reserve");

    assert!(is_replay);
    assert_eq!(r1.reservation_id, r2.reservation_id);
    assert_eq!(r1.event_id, r2.event_id);

    // Only one reservation row.
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inventory_reservations WHERE tenant_id = $1",
    )
    .bind(&tenant_id)
    .fetch_one(&pool)
    .await
    .expect("count");
    assert_eq!(count, 1, "no duplicate rows on replay");

    // quantity_reserved not doubled.
    let qty_reserved: i64 = sqlx::query_scalar(
        "SELECT quantity_reserved FROM item_on_hand WHERE tenant_id = $1 AND item_id = $2",
    )
    .bind(&tenant_id)
    .bind(item.id)
    .fetch_one(&pool)
    .await
    .expect("on-hand row");
    assert_eq!(qty_reserved, 50, "quantity_reserved must not be doubled");

    cleanup_tenant(&pool, &tenant_id).await;
}

#[tokio::test]
#[serial]
async fn inventory_reservation_idempotency_release() {
    let pool = get_inventory_pool().await;
    let tenant_id = format!("e2e-{}", Uuid::new_v4());
    let warehouse_id = Uuid::new_v4();

    let item = ItemRepo::create(&pool, &test_item_req(&tenant_id, "E2E-IDEM-REL-001"))
        .await
        .expect("create item");

    let (reserve_result, _) = process_reserve(&pool, &test_reserve_req(&tenant_id, item.id, warehouse_id))
        .await
        .expect("reserve");

    let rel_req = ReleaseRequest {
        tenant_id: tenant_id.clone(),
        reservation_id: reserve_result.reservation_id,
        idempotency_key: format!("e2e-rel-{}", Uuid::new_v4()),
        correlation_id: None,
        causation_id: None,
    };

    let (r1, _) = process_release(&pool, &rel_req).await.expect("first release");
    let (r2, is_replay) = process_release(&pool, &rel_req).await.expect("second release");

    assert!(is_replay);
    assert_eq!(r1.release_id, r2.release_id);
    assert_eq!(r1.event_id, r2.event_id);

    // Only one compensating row.
    let comp_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inventory_reservations WHERE reverses_reservation_id = $1",
    )
    .bind(reserve_result.reservation_id)
    .fetch_one(&pool)
    .await
    .expect("count");
    assert_eq!(comp_count, 1, "no duplicate compensating rows on replay");

    cleanup_tenant(&pool, &tenant_id).await;
}

#[tokio::test]
#[serial]
async fn inventory_reservation_double_release_rejected() {
    let pool = get_inventory_pool().await;
    let tenant_id = format!("e2e-{}", Uuid::new_v4());
    let warehouse_id = Uuid::new_v4();

    let item = ItemRepo::create(&pool, &test_item_req(&tenant_id, "E2E-DBL-REL-001"))
        .await
        .expect("create item");

    let (reserve_result, _) = process_reserve(&pool, &test_reserve_req(&tenant_id, item.id, warehouse_id))
        .await
        .expect("reserve");

    let rel_req1 = ReleaseRequest {
        tenant_id: tenant_id.clone(),
        reservation_id: reserve_result.reservation_id,
        idempotency_key: format!("e2e-rel-{}", Uuid::new_v4()),
        correlation_id: None,
        causation_id: None,
    };
    process_release(&pool, &rel_req1).await.expect("first release must succeed");

    let rel_req2 = ReleaseRequest {
        idempotency_key: format!("e2e-rel-{}", Uuid::new_v4()),
        ..rel_req1
    };
    let err = process_release(&pool, &rel_req2)
        .await
        .expect_err("double release must fail");

    assert!(
        matches!(err, ReservationError::AlreadyReleased),
        "expected AlreadyReleased, got: {:?}", err
    );

    cleanup_tenant(&pool, &tenant_id).await;
}

#[tokio::test]
#[serial]
async fn inventory_reservation_available_quantity() {
    let pool = get_inventory_pool().await;
    let tenant_id = format!("e2e-{}", Uuid::new_v4());
    let warehouse_id = Uuid::new_v4();

    let item = ItemRepo::create(&pool, &test_item_req(&tenant_id, "E2E-AVAIL-001"))
        .await
        .expect("create item");

    // Seed on-hand = 200.
    sqlx::query(
        r#"
        INSERT INTO item_on_hand (tenant_id, item_id, warehouse_id, quantity_on_hand, projected_at)
        VALUES ($1, $2, $3, 200, NOW())
        ON CONFLICT (tenant_id, item_id, warehouse_id) DO UPDATE
            SET quantity_on_hand = 200, projected_at = NOW()
        "#,
    )
    .bind(&tenant_id)
    .bind(item.id)
    .bind(warehouse_id)
    .execute(&pool)
    .await
    .expect("seed on-hand");

    // Reserve 75.
    let req = ReserveRequest {
        quantity: 75,
        ..test_reserve_req(&tenant_id, item.id, warehouse_id)
    };
    process_reserve(&pool, &req).await.expect("reserve 75");

    let (on_hand, reserved, available): (i64, i64, i64) = sqlx::query_as(
        "SELECT quantity_on_hand, quantity_reserved, quantity_available FROM item_on_hand WHERE tenant_id = $1 AND item_id = $2 AND warehouse_id = $3",
    )
    .bind(&tenant_id)
    .bind(item.id)
    .bind(warehouse_id)
    .fetch_one(&pool)
    .await
    .expect("on-hand row");

    assert_eq!(on_hand, 200, "on_hand must remain 200");
    assert_eq!(reserved, 75, "reserved must be 75");
    assert_eq!(available, 125, "available must equal on_hand - reserved = 125");

    cleanup_tenant(&pool, &tenant_id).await;
}
