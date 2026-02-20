//! E2E Test: Inventory Reservation Flow (bd-2y1a, bd-1xtm)
//!
//! ## Coverage (bd-2y1a)
//! 1. inventory_reservation_reserve_creates_atomically  — row + projection + outbox in one tx
//! 2. inventory_reservation_release_compensates         — compensating row references original
//! 3. inventory_reservation_idempotency_reserve         — duplicate key returns stored result
//! 4. inventory_reservation_idempotency_release         — duplicate release key is idempotent
//! 5. inventory_reservation_double_release_rejected     — AlreadyReleased guard
//! 6. inventory_reservation_available_quantity          — available = on_hand - reserved
//!
//! ## Coverage (bd-1xtm) — Lifecycle: reserve → fulfill + cancel
//! 7. inventory_reservation_fulfill_reduces_on_hand     — fulfill reduces quantity_on_hand
//! 8. inventory_reservation_fulfill_outbox_event        — fulfill writes outbox event
//! 9. inventory_reservation_cancel_releases_stock       — cancel returns stock to available
//! 10. inventory_reservation_fulfill_after_cancel_rejected — double-settle guard
//!
//! ## Setup
//! Set INVENTORY_DATABASE_URL (or DATABASE_URL) to the inventory DB connection string.

use inventory_rs::domain::{
    fulfill_service::{process_fulfill, FulfillRequest},
    items::{CreateItemRequest, ItemRepo, TrackingMode},
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
        tracking_mode: TrackingMode::None,
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
        INSERT INTO item_on_hand (tenant_id, item_id, warehouse_id, quantity_on_hand, available_status_on_hand, projected_at)
        VALUES ($1, $2, $3, 200, 200, NOW())
        ON CONFLICT (tenant_id, item_id, warehouse_id) WHERE location_id IS NULL DO UPDATE
            SET quantity_on_hand = 200, available_status_on_hand = 200, projected_at = NOW()
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

// ============================================================================
// bd-1xtm: Reservation lifecycle — fulfill + cancel
// ============================================================================

/// Helper: seed quantity_on_hand = n for a given item/warehouse.
async fn seed_on_hand(pool: &sqlx::PgPool, tenant_id: &str, item_id: uuid::Uuid, warehouse_id: uuid::Uuid, qty: i64) {
    sqlx::query(
        r#"
        INSERT INTO item_on_hand
            (tenant_id, item_id, warehouse_id, quantity_on_hand, available_status_on_hand, projected_at)
        VALUES ($1, $2, $3, $4, $4, NOW())
        ON CONFLICT (tenant_id, item_id, warehouse_id) WHERE location_id IS NULL DO UPDATE
            SET quantity_on_hand         = $4,
                available_status_on_hand = $4,
                projected_at             = NOW()
        "#,
    )
    .bind(tenant_id)
    .bind(item_id)
    .bind(warehouse_id)
    .bind(qty)
    .execute(pool)
    .await
    .expect("seed on_hand");
}

/// bd-1xtm step 1-5: reserve 10, verify available=90, fulfill, verify on_hand=90.
///
/// Start: quantity_on_hand=100, reserved=0, available=100
/// Reserve 10: reserved=10, available=90
/// Fulfill 10: on_hand=90, reserved=0, available=90
#[tokio::test]
#[serial]
async fn inventory_reservation_fulfill_reduces_on_hand() {
    let pool = get_inventory_pool().await;
    let tenant_id = format!("e2e-{}", Uuid::new_v4());
    let warehouse_id = Uuid::new_v4();

    let item = ItemRepo::create(&pool, &test_item_req(&tenant_id, "E2E-FULF-001"))
        .await
        .expect("create item");

    // Seed 100 units on hand.
    seed_on_hand(&pool, &tenant_id, item.id, warehouse_id, 100).await;

    // Reserve 10 with an order reference.
    let reserve_req = ReserveRequest {
        quantity: 10,
        reference_type: Some("fulfillment_order".to_string()),
        reference_id: Some("order-123".to_string()),
        ..test_reserve_req(&tenant_id, item.id, warehouse_id)
    };
    let (reserve_result, _) = process_reserve(&pool, &reserve_req)
        .await
        .expect("reserve 10 must succeed");

    // Verify available reduced to 90 (on_hand=100, reserved=10, available=90).
    let (on_hand, reserved, available): (i64, i64, i64) = sqlx::query_as(
        "SELECT quantity_on_hand, quantity_reserved, quantity_available FROM item_on_hand \
         WHERE tenant_id = $1 AND item_id = $2 AND warehouse_id = $3",
    )
    .bind(&tenant_id)
    .bind(item.id)
    .bind(warehouse_id)
    .fetch_one(&pool)
    .await
    .expect("on-hand row after reserve");

    assert_eq!(on_hand, 100, "on_hand unchanged after reserve");
    assert_eq!(reserved, 10, "reserved = 10 after reserve");
    assert_eq!(available, 90, "available = 90 after reserve");

    // Fulfill the reservation.
    let fulfill_req = FulfillRequest {
        tenant_id: tenant_id.clone(),
        reservation_id: reserve_result.reservation_id,
        quantity: None, // fulfill full reserved quantity
        order_ref: Some("order-123".to_string()),
        idempotency_key: format!("e2e-fulf-{}", Uuid::new_v4()),
        correlation_id: None,
        causation_id: None,
    };
    let (fulfill_result, is_replay) = process_fulfill(&pool, &fulfill_req)
        .await
        .expect("fulfill must succeed");

    assert!(!is_replay, "must be a new fulfillment");
    assert_eq!(fulfill_result.reservation_id, reserve_result.reservation_id);
    assert_eq!(fulfill_result.quantity, 10);
    assert_eq!(fulfill_result.order_ref.as_deref(), Some("order-123"));

    // Verify compensating row: status='fulfilled', links to original.
    let (status, reverses): (String, Option<Uuid>) = sqlx::query_as(
        "SELECT status::TEXT, reverses_reservation_id FROM inventory_reservations WHERE id = $1",
    )
    .bind(fulfill_result.fulfillment_id)
    .fetch_one(&pool)
    .await
    .expect("fulfillment row");

    assert_eq!(status, "fulfilled");
    assert_eq!(reverses, Some(reserve_result.reservation_id),
        "compensating row must reference original reserve");

    // Verify on_hand=90, reserved=0, available=90 after fulfillment.
    let (on_hand, reserved, available): (i64, i64, i64) = sqlx::query_as(
        "SELECT quantity_on_hand, quantity_reserved, quantity_available FROM item_on_hand \
         WHERE tenant_id = $1 AND item_id = $2 AND warehouse_id = $3",
    )
    .bind(&tenant_id)
    .bind(item.id)
    .bind(warehouse_id)
    .fetch_one(&pool)
    .await
    .expect("on-hand row after fulfill");

    assert_eq!(on_hand, 90, "on_hand must be 90 after fulfill (stock physically issued)");
    assert_eq!(reserved, 0, "reserved must be 0 after fulfill (hold consumed)");
    assert_eq!(available, 90, "available must remain 90 after fulfill");

    cleanup_tenant(&pool, &tenant_id).await;
}

/// bd-1xtm step 6: verify outbox event written for fulfillment.
#[tokio::test]
#[serial]
async fn inventory_reservation_fulfill_outbox_event() {
    let pool = get_inventory_pool().await;
    let tenant_id = format!("e2e-{}", Uuid::new_v4());
    let warehouse_id = Uuid::new_v4();

    let item = ItemRepo::create(&pool, &test_item_req(&tenant_id, "E2E-FULF-EVT-001"))
        .await
        .expect("create item");

    seed_on_hand(&pool, &tenant_id, item.id, warehouse_id, 50).await;

    let (reserve_result, _) = process_reserve(
        &pool,
        &ReserveRequest {
            quantity: 20,
            reference_type: Some("sales_order".to_string()),
            reference_id: Some("SO-E2E-99".to_string()),
            ..test_reserve_req(&tenant_id, item.id, warehouse_id)
        },
    )
    .await
    .expect("reserve");

    let (fulfill_result, _) = process_fulfill(
        &pool,
        &FulfillRequest {
            tenant_id: tenant_id.clone(),
            reservation_id: reserve_result.reservation_id,
            quantity: None,
            order_ref: Some("SO-E2E-99".to_string()),
            idempotency_key: format!("e2e-fulf-evt-{}", Uuid::new_v4()),
            correlation_id: None,
            causation_id: None,
        },
    )
    .await
    .expect("fulfill");

    // Verify outbox event was written with correct type and payload.
    let (event_type, payload_text): (String, String) = sqlx::query_as(
        "SELECT event_type, payload::TEXT FROM inv_outbox WHERE event_id = $1",
    )
    .bind(fulfill_result.event_id)
    .fetch_one(&pool)
    .await
    .expect("outbox event");

    assert_eq!(event_type, "inventory.reservation_fulfilled");

    let payload: serde_json::Value = serde_json::from_str(&payload_text).expect("parse payload");
    assert_eq!(
        payload["item_id"].as_str().unwrap(),
        item.id.to_string(),
        "payload must include item_id"
    );
    assert_eq!(payload["quantity"], 20, "payload must include quantity");
    assert_eq!(
        payload["order_ref"].as_str().unwrap(),
        "SO-E2E-99",
        "payload must include order_ref"
    );

    cleanup_tenant(&pool, &tenant_id).await;
}

/// bd-1xtm step 7: cancel a reservation — verify stock released back to available.
///
/// Start: on_hand=100, reserved=15, available=85
/// Cancel (release): reserved=0, available=100
/// on_hand stays at 100 (no physical movement)
#[tokio::test]
#[serial]
async fn inventory_reservation_cancel_releases_stock() {
    let pool = get_inventory_pool().await;
    let tenant_id = format!("e2e-{}", Uuid::new_v4());
    let warehouse_id = Uuid::new_v4();

    let item = ItemRepo::create(&pool, &test_item_req(&tenant_id, "E2E-CANCEL-001"))
        .await
        .expect("create item");

    seed_on_hand(&pool, &tenant_id, item.id, warehouse_id, 100).await;

    // Reserve 15 units.
    let (reserve_result, _) = process_reserve(
        &pool,
        &ReserveRequest {
            quantity: 15,
            reference_type: Some("sales_order".to_string()),
            reference_id: Some("SO-CANCEL-01".to_string()),
            ..test_reserve_req(&tenant_id, item.id, warehouse_id)
        },
    )
    .await
    .expect("reserve 15");

    // Verify available reduced to 85.
    let available: i64 = sqlx::query_scalar(
        "SELECT quantity_available FROM item_on_hand WHERE tenant_id = $1 AND item_id = $2 AND warehouse_id = $3",
    )
    .bind(&tenant_id)
    .bind(item.id)
    .bind(warehouse_id)
    .fetch_one(&pool)
    .await
    .expect("on-hand available after reserve");

    assert_eq!(available, 85, "available must be 85 after reserving 15 from 100");

    // Cancel the reservation (release).
    let (release_result, _) = process_release(
        &pool,
        &ReleaseRequest {
            tenant_id: tenant_id.clone(),
            reservation_id: reserve_result.reservation_id,
            idempotency_key: format!("e2e-cancel-{}", Uuid::new_v4()),
            correlation_id: None,
            causation_id: None,
        },
    )
    .await
    .expect("cancel (release) must succeed");

    assert_eq!(release_result.reservation_id, reserve_result.reservation_id);

    // Verify stock returned: on_hand=100 (unchanged), reserved=0, available=100.
    let (on_hand, reserved, available): (i64, i64, i64) = sqlx::query_as(
        "SELECT quantity_on_hand, quantity_reserved, quantity_available FROM item_on_hand \
         WHERE tenant_id = $1 AND item_id = $2 AND warehouse_id = $3",
    )
    .bind(&tenant_id)
    .bind(item.id)
    .bind(warehouse_id)
    .fetch_one(&pool)
    .await
    .expect("on-hand row after cancel");

    assert_eq!(on_hand, 100, "on_hand unchanged after cancel (no physical movement)");
    assert_eq!(reserved, 0, "reserved must be 0 after cancel");
    assert_eq!(available, 100, "available fully restored after cancel");

    cleanup_tenant(&pool, &tenant_id).await;
}

/// bd-1xtm: fulfill after cancel (or cancel after fulfill) must be rejected.
///
/// Guard: a reservation can only be settled once (either released or fulfilled).
/// Attempting to fulfill an already-cancelled reservation must fail with AlreadySettled.
#[tokio::test]
#[serial]
async fn inventory_reservation_fulfill_after_cancel_rejected() {
    let pool = get_inventory_pool().await;
    let tenant_id = format!("e2e-{}", Uuid::new_v4());
    let warehouse_id = Uuid::new_v4();

    let item = ItemRepo::create(&pool, &test_item_req(&tenant_id, "E2E-DBL-SETTLE-001"))
        .await
        .expect("create item");

    seed_on_hand(&pool, &tenant_id, item.id, warehouse_id, 50).await;

    let (reserve_result, _) = process_reserve(
        &pool,
        &ReserveRequest {
            quantity: 5,
            ..test_reserve_req(&tenant_id, item.id, warehouse_id)
        },
    )
    .await
    .expect("reserve");

    // Cancel (release) first.
    process_release(
        &pool,
        &ReleaseRequest {
            tenant_id: tenant_id.clone(),
            reservation_id: reserve_result.reservation_id,
            idempotency_key: format!("e2e-cancel-first-{}", Uuid::new_v4()),
            correlation_id: None,
            causation_id: None,
        },
    )
    .await
    .expect("cancel must succeed");

    // Attempt to fulfill an already-cancelled reservation — must fail.
    use inventory_rs::domain::fulfill_service::FulfillError;
    let err = process_fulfill(
        &pool,
        &FulfillRequest {
            tenant_id: tenant_id.clone(),
            reservation_id: reserve_result.reservation_id,
            quantity: None,
            order_ref: None,
            idempotency_key: format!("e2e-fulf-late-{}", Uuid::new_v4()),
            correlation_id: None,
            causation_id: None,
        },
    )
    .await
    .expect_err("fulfill after cancel must fail");

    assert!(
        matches!(err, FulfillError::AlreadySettled),
        "expected AlreadySettled, got: {:?}", err
    );

    cleanup_tenant(&pool, &tenant_id).await;
}
