//! Integration tests for the reservation flow (bd-2y1a).
//!
//! Tests run against a real PostgreSQL database.
//! Set DATABASE_URL to the inventory database connection string.
//!
//! Coverage:
//! 1. Reserve creates reservation row + on-hand projection updated + outbox event
//! 2. Release creates compensating row referencing original + projection decremented
//! 3. Release compensating row references original (compensating linkage)
//! 4. Idempotency: duplicate reserve key returns stored result, no extra rows
//! 5. Idempotency: duplicate release key returns stored result, no extra rows
//! 6. Guard: inactive item rejected on reserve
//! 7. Guard: double-release rejected (AlreadyReleased)
//! 8. Guard: unknown reservation_id rejected on release
//! 9. Available = on_hand - reserved (projection correctness)

use inventory_rs::domain::{
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

fn make_item_req(tenant_id: &str, sku: &str) -> CreateItemRequest {
    CreateItemRequest {
        tenant_id: tenant_id.to_string(),
        sku: sku.to_string(),
        name: "Reservation Test Item".to_string(),
        description: None,
        inventory_account_ref: "1200".to_string(),
        cogs_account_ref: "5000".to_string(),
        variance_account_ref: "5010".to_string(),
        uom: None,
        tracking_mode: TrackingMode::None,
        make_buy: None,
    }
}

fn make_reserve_req(
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
    qty: i64,
) -> ReserveRequest {
    ReserveRequest {
        tenant_id: tenant_id.to_string(),
        item_id,
        warehouse_id,
        quantity: qty,
        reference_type: Some("sales_order".to_string()),
        reference_id: Some(format!("SO-{}", Uuid::new_v4())),
        expires_at: None,
        idempotency_key: format!("res-{}", Uuid::new_v4()),
        correlation_id: Some("corr-test".to_string()),
        causation_id: None,
    }
}

async fn cleanup_tenant(pool: &sqlx::PgPool, tenant_id: &str) {
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
    sqlx::query("DELETE FROM inventory_reservations WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM item_on_hand_by_status WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM item_on_hand WHERE tenant_id = $1")
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
    sqlx::query("DELETE FROM items WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
}

// ============================================================================
// Test 1: Reserve creates reservation row + projection + outbox atomically
// ============================================================================

#[tokio::test]
#[serial]
async fn reserve_creates_reservation_projection_outbox_atomically() {
    let pool = setup_db().await;
    let tenant_id = format!("test-{}", Uuid::new_v4());
    let warehouse_id = Uuid::new_v4();

    let item = ItemRepo::create(&pool, &make_item_req(&tenant_id, "SKU-RSV-001"))
        .await
        .expect("create item");

    let req = make_reserve_req(&tenant_id, item.id, warehouse_id, 25);
    let (result, is_replay) = process_reserve(&pool, &req)
        .await
        .expect("reserve must succeed");

    assert!(!is_replay, "first call must not be replay");
    assert_eq!(result.quantity, 25);
    assert_eq!(result.tenant_id, tenant_id);

    // Reservation row exists with correct status and no compensating linkage.
    let (status, reverses_id): (String, Option<Uuid>) = sqlx::query_as(
        "SELECT status::TEXT, reverses_reservation_id FROM inventory_reservations WHERE id = $1",
    )
    .bind(result.reservation_id)
    .fetch_one(&pool)
    .await
    .expect("reservation row must exist");

    assert_eq!(status, "active");
    assert!(
        reverses_id.is_none(),
        "primary reserve row must have NULL reverses_reservation_id"
    );

    // On-hand projection updated: quantity_reserved = 25.
    let qty_reserved: i64 = sqlx::query_scalar(
        "SELECT quantity_reserved FROM item_on_hand WHERE tenant_id = $1 AND item_id = $2 AND warehouse_id = $3",
    )
    .bind(&tenant_id)
    .bind(item.id)
    .bind(warehouse_id)
    .fetch_one(&pool)
    .await
    .expect("on-hand row must exist");

    assert_eq!(qty_reserved, 25);

    // Outbox event exists with correct type.
    let event_type: String =
        sqlx::query_scalar("SELECT event_type FROM inv_outbox WHERE event_id = $1")
            .bind(result.event_id)
            .fetch_one(&pool)
            .await
            .expect("outbox event must exist");

    assert_eq!(event_type, "inventory.item_reserved");

    cleanup_tenant(&pool, &tenant_id).await;
}

// ============================================================================
// Test 2: Release creates compensating row and decrements projection
// ============================================================================

#[tokio::test]
#[serial]
async fn release_creates_compensating_row_and_decrements_projection() {
    let pool = setup_db().await;
    let tenant_id = format!("test-{}", Uuid::new_v4());
    let warehouse_id = Uuid::new_v4();

    let item = ItemRepo::create(&pool, &make_item_req(&tenant_id, "SKU-REL-001"))
        .await
        .expect("create item");

    // Reserve first.
    let reserve_req = make_reserve_req(&tenant_id, item.id, warehouse_id, 30);
    let (reserve_result, _) = process_reserve(&pool, &reserve_req)
        .await
        .expect("reserve must succeed");

    // Release it.
    let release_req = ReleaseRequest {
        tenant_id: tenant_id.clone(),
        reservation_id: reserve_result.reservation_id,
        idempotency_key: format!("rel-{}", Uuid::new_v4()),
        correlation_id: None,
        causation_id: None,
    };
    let (release_result, is_replay) = process_release(&pool, &release_req)
        .await
        .expect("release must succeed");

    assert!(!is_replay);
    assert_eq!(release_result.reservation_id, reserve_result.reservation_id);

    // Compensating row exists with correct linkage and status.
    let (status, reverses_id): (String, Option<Uuid>) = sqlx::query_as(
        "SELECT status::TEXT, reverses_reservation_id FROM inventory_reservations WHERE id = $1",
    )
    .bind(release_result.release_id)
    .fetch_one(&pool)
    .await
    .expect("release row must exist");

    assert_eq!(status, "released");
    assert_eq!(
        reverses_id,
        Some(reserve_result.reservation_id),
        "compensating row must reference original reserve"
    );

    // Projection quantity_reserved decremented back to 0.
    let qty_reserved: i64 = sqlx::query_scalar(
        "SELECT quantity_reserved FROM item_on_hand WHERE tenant_id = $1 AND item_id = $2 AND warehouse_id = $3",
    )
    .bind(&tenant_id)
    .bind(item.id)
    .bind(warehouse_id)
    .fetch_one(&pool)
    .await
    .expect("on-hand row must exist");

    assert_eq!(
        qty_reserved, 0,
        "quantity_reserved should be 0 after release"
    );

    // Outbox event for release.
    let event_type: String =
        sqlx::query_scalar("SELECT event_type FROM inv_outbox WHERE event_id = $1")
            .bind(release_result.event_id)
            .fetch_one(&pool)
            .await
            .expect("outbox event must exist");

    assert_eq!(event_type, "inventory.reservation_released");

    cleanup_tenant(&pool, &tenant_id).await;
}

// ============================================================================
// Test 3: Reserve idempotency replay returns stored result, no extra rows
// ============================================================================

#[tokio::test]
#[serial]
async fn reserve_idempotency_replay_returns_stored_result() {
    let pool = setup_db().await;
    let tenant_id = format!("test-{}", Uuid::new_v4());
    let warehouse_id = Uuid::new_v4();

    let item = ItemRepo::create(&pool, &make_item_req(&tenant_id, "SKU-IDEM-RSV-001"))
        .await
        .expect("create item");

    let req = make_reserve_req(&tenant_id, item.id, warehouse_id, 10);

    let (r1, is_replay1) = process_reserve(&pool, &req).await.expect("first call");
    assert!(!is_replay1);

    let (r2, is_replay2) = process_reserve(&pool, &req).await.expect("second call");
    assert!(is_replay2, "second call must be a replay");

    assert_eq!(r1.reservation_id, r2.reservation_id);
    assert_eq!(r1.event_id, r2.event_id);
    assert_eq!(r1.quantity, r2.quantity);

    // No duplicate reservation rows.
    let row_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM inventory_reservations WHERE tenant_id = $1")
            .bind(&tenant_id)
            .fetch_one(&pool)
            .await
            .expect("count");
    assert_eq!(row_count, 1, "no duplicate reservation rows on replay");

    // quantity_reserved not doubled.
    let qty_reserved: i64 = sqlx::query_scalar(
        "SELECT quantity_reserved FROM item_on_hand WHERE tenant_id = $1 AND item_id = $2",
    )
    .bind(&tenant_id)
    .bind(item.id)
    .fetch_one(&pool)
    .await
    .expect("on-hand row");
    assert_eq!(
        qty_reserved, 10,
        "quantity_reserved must not be doubled on replay"
    );

    cleanup_tenant(&pool, &tenant_id).await;
}

// ============================================================================
// Test 4: Release idempotency replay
// ============================================================================

#[tokio::test]
#[serial]
async fn release_idempotency_replay_returns_stored_result() {
    let pool = setup_db().await;
    let tenant_id = format!("test-{}", Uuid::new_v4());
    let warehouse_id = Uuid::new_v4();

    let item = ItemRepo::create(&pool, &make_item_req(&tenant_id, "SKU-IDEM-REL-001"))
        .await
        .expect("create item");

    let reserve_req = make_reserve_req(&tenant_id, item.id, warehouse_id, 10);
    let (reserve_result, _) = process_reserve(&pool, &reserve_req).await.expect("reserve");

    let release_req = ReleaseRequest {
        tenant_id: tenant_id.clone(),
        reservation_id: reserve_result.reservation_id,
        idempotency_key: format!("rel-idem-{}", Uuid::new_v4()),
        correlation_id: None,
        causation_id: None,
    };

    let (r1, _) = process_release(&pool, &release_req)
        .await
        .expect("first release");
    let (r2, is_replay) = process_release(&pool, &release_req)
        .await
        .expect("second release");

    assert!(is_replay, "second release must be a replay");
    assert_eq!(r1.release_id, r2.release_id);
    assert_eq!(r1.event_id, r2.event_id);

    // Only one compensating row.
    let compensating_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inventory_reservations WHERE reverses_reservation_id = $1",
    )
    .bind(reserve_result.reservation_id)
    .fetch_one(&pool)
    .await
    .expect("count");
    assert_eq!(
        compensating_count, 1,
        "no duplicate compensating rows on replay"
    );

    cleanup_tenant(&pool, &tenant_id).await;
}

// ============================================================================
// Test 5: Guard — inactive item rejected on reserve
// ============================================================================

#[tokio::test]
#[serial]
async fn reserve_guard_rejects_inactive_item() {
    let pool = setup_db().await;
    let tenant_id = format!("test-{}", Uuid::new_v4());

    let item = ItemRepo::create(&pool, &make_item_req(&tenant_id, "SKU-INACT-001"))
        .await
        .expect("create item");

    ItemRepo::deactivate(&pool, item.id, &tenant_id)
        .await
        .expect("deactivate");

    let req = make_reserve_req(&tenant_id, item.id, Uuid::new_v4(), 5);
    let err = process_reserve(&pool, &req)
        .await
        .expect_err("inactive item must be rejected");

    assert!(
        matches!(err, ReservationError::Guard(_)),
        "expected Guard error, got: {:?}",
        err
    );

    cleanup_tenant(&pool, &tenant_id).await;
}

// ============================================================================
// Test 6: Guard — double release rejected (AlreadyReleased)
// ============================================================================

#[tokio::test]
#[serial]
async fn release_guard_rejects_double_release() {
    let pool = setup_db().await;
    let tenant_id = format!("test-{}", Uuid::new_v4());
    let warehouse_id = Uuid::new_v4();

    let item = ItemRepo::create(&pool, &make_item_req(&tenant_id, "SKU-DBL-REL-001"))
        .await
        .expect("create item");

    let reserve_req = make_reserve_req(&tenant_id, item.id, warehouse_id, 15);
    let (reserve_result, _) = process_reserve(&pool, &reserve_req).await.expect("reserve");

    // First release succeeds.
    let release_req1 = ReleaseRequest {
        tenant_id: tenant_id.clone(),
        reservation_id: reserve_result.reservation_id,
        idempotency_key: format!("rel-{}", Uuid::new_v4()),
        correlation_id: None,
        causation_id: None,
    };
    process_release(&pool, &release_req1)
        .await
        .expect("first release must succeed");

    // Second release with DIFFERENT idempotency key — must be rejected.
    let release_req2 = ReleaseRequest {
        idempotency_key: format!("rel-{}", Uuid::new_v4()),
        ..release_req1
    };
    let err = process_release(&pool, &release_req2)
        .await
        .expect_err("double release must fail");

    assert!(
        matches!(err, ReservationError::AlreadyReleased),
        "expected AlreadyReleased, got: {:?}",
        err
    );

    cleanup_tenant(&pool, &tenant_id).await;
}

// ============================================================================
// Test 7: Guard — unknown reservation_id rejected on release
// ============================================================================

#[tokio::test]
#[serial]
async fn release_guard_rejects_unknown_reservation() {
    let pool = setup_db().await;
    let tenant_id = format!("test-{}", Uuid::new_v4());

    let release_req = ReleaseRequest {
        tenant_id: tenant_id.clone(),
        reservation_id: Uuid::new_v4(), // does not exist
        idempotency_key: format!("rel-{}", Uuid::new_v4()),
        correlation_id: None,
        causation_id: None,
    };

    let err = process_release(&pool, &release_req)
        .await
        .expect_err("unknown reservation must fail");

    assert!(
        matches!(err, ReservationError::ReservationNotFound),
        "expected ReservationNotFound, got: {:?}",
        err
    );
}

// ============================================================================
// Test 8: Available = on_hand - reserved (projection correctness)
// ============================================================================

#[tokio::test]
#[serial]
async fn available_equals_on_hand_minus_reserved() {
    let pool = setup_db().await;
    let tenant_id = format!("test-{}", Uuid::new_v4());
    let warehouse_id = Uuid::new_v4();

    let item = ItemRepo::create(&pool, &make_item_req(&tenant_id, "SKU-AVAIL-001"))
        .await
        .expect("create item");

    // Seed: manually set on-hand = 100 (all in 'available' status).
    sqlx::query(
        r#"
        INSERT INTO item_on_hand
            (tenant_id, item_id, warehouse_id, quantity_on_hand, available_status_on_hand, projected_at)
        VALUES ($1, $2, $3, 100, 100, NOW())
        ON CONFLICT (tenant_id, item_id, warehouse_id) WHERE location_id IS NULL DO UPDATE
            SET quantity_on_hand = 100, available_status_on_hand = 100, projected_at = NOW()
        "#,
    )
    .bind(&tenant_id)
    .bind(item.id)
    .bind(warehouse_id)
    .execute(&pool)
    .await
    .expect("seed on-hand");

    // Reserve 40 units.
    let req = make_reserve_req(&tenant_id, item.id, warehouse_id, 40);
    process_reserve(&pool, &req).await.expect("reserve 40");

    // Check available = 100 - 40 = 60.
    let (on_hand, reserved, available): (i64, i64, i64) = sqlx::query_as(
        "SELECT quantity_on_hand, quantity_reserved, quantity_available FROM item_on_hand WHERE tenant_id = $1 AND item_id = $2 AND warehouse_id = $3",
    )
    .bind(&tenant_id)
    .bind(item.id)
    .bind(warehouse_id)
    .fetch_one(&pool)
    .await
    .expect("on-hand row");

    assert_eq!(on_hand, 100);
    assert_eq!(reserved, 40);
    assert_eq!(available, 60, "available must equal on_hand - reserved");

    cleanup_tenant(&pool, &tenant_id).await;
}

// ============================================================================
// Test 9: Guard — zero quantity rejected on reserve (stateless)
// ============================================================================

#[tokio::test]
async fn reserve_guard_rejects_zero_quantity() {
    let pool = setup_db().await;
    let tenant_id = format!("test-{}", Uuid::new_v4());

    let req = ReserveRequest {
        quantity: 0,
        ..make_reserve_req(&tenant_id, Uuid::new_v4(), Uuid::new_v4(), 1)
    };

    let err = process_reserve(&pool, &req)
        .await
        .expect_err("zero quantity must fail");
    assert!(matches!(err, ReservationError::Guard(_)), "{:?}", err);
}
