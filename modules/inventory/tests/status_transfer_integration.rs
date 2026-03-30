//! Integration tests for status bucket transfer (bd-3s57).
//!
//! Tests run against a real PostgreSQL database.
//! Set DATABASE_URL to the inventory database connection string.
//!
//! Coverage:
//! 1. Happy path: available → quarantine decrements available, increments quarantine
//! 2. Idempotency: duplicate key returns stored result without double-transfer
//! 3. Guard: insufficient available stock (respects reservations)
//! 4. Guard: same from/to status rejected
//! 5. Guard: zero quantity rejected
//! 6. Guard: inactive item rejected
//! 7. Non-available bucket transfer: quarantine → damaged
//! 8. Return to available: quarantine → available increments available_status_on_hand
//! 9. Idempotency key conflict: same key, different body → 409-equivalent error

use inventory_rs::domain::{
    items::{CreateItemRequest, ItemRepo, TrackingMode},
    receipt_service::{process_receipt, ReceiptRequest},
    status::{
        models::InvItemStatus,
        transfer_service::{process_status_transfer, StatusTransferError, StatusTransferRequest},
    },
};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

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

fn create_item_req(tenant_id: &str, sku: &str) -> CreateItemRequest {
    CreateItemRequest {
        tenant_id: tenant_id.to_string(),
        sku: sku.to_string(),
        name: "Status Test Widget".to_string(),
        description: None,
        inventory_account_ref: "1200".to_string(),
        cogs_account_ref: "5000".to_string(),
        variance_account_ref: "5010".to_string(),
        uom: None,
        tracking_mode: TrackingMode::None,
        make_buy: None,
    }
}

fn receipt_req(
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
    qty: i64,
    idem: &str,
) -> ReceiptRequest {
    ReceiptRequest {
        tenant_id: tenant_id.to_string(),
        item_id,
        warehouse_id,
        location_id: None,
        quantity: qty,
        unit_cost_minor: 1_000,
        currency: "usd".to_string(),
        source_type: "purchase".to_string(),
        purchase_order_id: None,
        idempotency_key: idem.to_string(),
        correlation_id: None,
        causation_id: None,
        lot_code: None,
        serial_codes: None,
        uom_id: None,
    }
}

fn transfer_req(
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
    from: InvItemStatus,
    to: InvItemStatus,
    qty: i64,
    idem: &str,
) -> StatusTransferRequest {
    StatusTransferRequest {
        tenant_id: tenant_id.to_string(),
        item_id,
        warehouse_id,
        from_status: from,
        to_status: to,
        quantity: qty,
        idempotency_key: idem.to_string(),
        correlation_id: None,
        causation_id: None,
    }
}

// Reads status bucket quantities for an item/warehouse.
async fn read_buckets(
    pool: &sqlx::PgPool,
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
) -> std::collections::HashMap<String, i64> {
    #[derive(sqlx::FromRow)]
    struct Row {
        status: String,
        quantity_on_hand: i64,
    }

    let rows = sqlx::query_as::<_, Row>(
        "SELECT status::TEXT AS status, quantity_on_hand \
         FROM item_on_hand_by_status \
         WHERE tenant_id = $1 AND item_id = $2 AND warehouse_id = $3",
    )
    .bind(tenant_id)
    .bind(item_id)
    .bind(warehouse_id)
    .fetch_all(pool)
    .await
    .expect("read_buckets failed");

    rows.into_iter()
        .map(|r| (r.status, r.quantity_on_hand))
        .collect()
}

async fn read_available_on_hand(
    pool: &sqlx::PgPool,
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "SELECT available_status_on_hand \
         FROM item_on_hand \
         WHERE tenant_id = $1 AND item_id = $2 AND warehouse_id = $3 AND location_id IS NULL",
    )
    .bind(tenant_id)
    .bind(item_id)
    .bind(warehouse_id)
    .fetch_one(pool)
    .await
    .expect("read_available_on_hand failed")
}

#[tokio::test]
#[serial]
async fn test_transfer_available_to_quarantine() {
    let pool = setup_db().await;
    let tenant = format!("st-t1-{}", Uuid::new_v4().simple());
    let wh = Uuid::new_v4();
    let item = ItemRepo::create(&pool, &create_item_req(&tenant, "ST-001"))
        .await
        .expect("create item");

    // Receipt 100 units → all go to 'available'
    process_receipt(&pool, &receipt_req(&tenant, item.id, wh, 100, "r-1"), None)
        .await
        .expect("receipt");

    // Transfer 30 from available to quarantine
    let (result, is_replay) = process_status_transfer(
        &pool,
        &transfer_req(
            &tenant,
            item.id,
            wh,
            InvItemStatus::Available,
            InvItemStatus::Quarantine,
            30,
            "tx-1",
        ),
    )
    .await
    .expect("transfer");

    assert!(!is_replay);
    assert_eq!(result.from_status, "available");
    assert_eq!(result.to_status, "quarantine");
    assert_eq!(result.quantity, 30);

    // Verify buckets
    let buckets = read_buckets(&pool, &tenant, item.id, wh).await;
    assert_eq!(buckets["available"], 70);
    assert_eq!(buckets["quarantine"], 30);

    // Verify item_on_hand.available_status_on_hand kept in sync
    let avail_oh = read_available_on_hand(&pool, &tenant, item.id, wh).await;
    assert_eq!(avail_oh, 70);

    // Verify outbox has the event
    let outbox_count: i64 = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM inv_outbox WHERE tenant_id = $1 AND event_id = $2",
    )
    .bind(&tenant)
    .bind(result.event_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(outbox_count, 1);

    // Verify ledger row
    let ledger_count: i64 = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM inv_status_transfers WHERE tenant_id = $1 AND id = $2",
    )
    .bind(&tenant)
    .bind(result.transfer_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(ledger_count, 1);
}

#[tokio::test]
#[serial]
async fn test_transfer_idempotency() {
    let pool = setup_db().await;
    let tenant = format!("st-t2-{}", Uuid::new_v4().simple());
    let wh = Uuid::new_v4();
    let item = ItemRepo::create(&pool, &create_item_req(&tenant, "ST-002"))
        .await
        .expect("create item");

    process_receipt(&pool, &receipt_req(&tenant, item.id, wh, 100, "r-2"), None)
        .await
        .expect("receipt");

    let req = transfer_req(
        &tenant,
        item.id,
        wh,
        InvItemStatus::Available,
        InvItemStatus::Quarantine,
        20,
        "tx-idem",
    );

    let (r1, replay1) = process_status_transfer(&pool, &req)
        .await
        .expect("transfer 1");
    assert!(!replay1);

    // Second call — same idempotency key
    let (r2, replay2) = process_status_transfer(&pool, &req)
        .await
        .expect("transfer 2");
    assert!(replay2);
    assert_eq!(r1.transfer_id, r2.transfer_id);
    assert_eq!(r1.event_id, r2.event_id);

    // Buckets should NOT have been debited twice
    let buckets = read_buckets(&pool, &tenant, item.id, wh).await;
    assert_eq!(buckets["available"], 80); // only decremented once
    assert_eq!(buckets["quarantine"], 20);
}

#[tokio::test]
#[serial]
async fn test_transfer_insufficient_available() {
    let pool = setup_db().await;
    let tenant = format!("st-t3-{}", Uuid::new_v4().simple());
    let wh = Uuid::new_v4();
    let item = ItemRepo::create(&pool, &create_item_req(&tenant, "ST-003"))
        .await
        .expect("create item");

    // Only 10 units in stock
    process_receipt(&pool, &receipt_req(&tenant, item.id, wh, 10, "r-3"), None)
        .await
        .expect("receipt");

    // Try to transfer 20 (more than available)
    let err = process_status_transfer(
        &pool,
        &transfer_req(
            &tenant,
            item.id,
            wh,
            InvItemStatus::Available,
            InvItemStatus::Quarantine,
            20,
            "tx-insuf",
        ),
    )
    .await
    .expect_err("should fail");

    assert!(
        matches!(err, StatusTransferError::InsufficientStock { .. }),
        "expected InsufficientStock, got {:?}",
        err
    );

    // Buckets should be unchanged
    let buckets = read_buckets(&pool, &tenant, item.id, wh).await;
    assert_eq!(buckets["available"], 10);
    assert!(!buckets.contains_key("quarantine"));
}

#[tokio::test]
#[serial]
async fn test_transfer_same_status_rejected() {
    let pool = setup_db().await;
    let tenant = format!("st-t4-{}", Uuid::new_v4().simple());
    let wh = Uuid::new_v4();
    let item = ItemRepo::create(&pool, &create_item_req(&tenant, "ST-004"))
        .await
        .expect("create item");

    let err = process_status_transfer(
        &pool,
        &transfer_req(
            &tenant,
            item.id,
            wh,
            InvItemStatus::Available,
            InvItemStatus::Available,
            5,
            "tx-same",
        ),
    )
    .await
    .expect_err("should fail");

    assert!(
        matches!(err, StatusTransferError::SameStatus),
        "expected SameStatus, got {:?}",
        err
    );
}

#[tokio::test]
#[serial]
async fn test_transfer_zero_quantity_rejected() {
    let pool = setup_db().await;
    let tenant = format!("st-t5-{}", Uuid::new_v4().simple());
    let wh = Uuid::new_v4();
    let item = ItemRepo::create(&pool, &create_item_req(&tenant, "ST-005"))
        .await
        .expect("create item");

    let mut req = transfer_req(
        &tenant,
        item.id,
        wh,
        InvItemStatus::Available,
        InvItemStatus::Quarantine,
        0,
        "tx-zero",
    );
    req.quantity = 0;

    let err = process_status_transfer(&pool, &req)
        .await
        .expect_err("should fail");

    assert!(
        matches!(err, StatusTransferError::Guard(_)),
        "expected Guard error, got {:?}",
        err
    );
}

#[tokio::test]
#[serial]
async fn test_transfer_inactive_item_rejected() {
    let pool = setup_db().await;
    let tenant = format!("st-t6-{}", Uuid::new_v4().simple());
    let wh = Uuid::new_v4();
    let item = ItemRepo::create(&pool, &create_item_req(&tenant, "ST-006"))
        .await
        .expect("create item");

    // Deactivate the item
    sqlx::query("UPDATE items SET active = false WHERE id = $1")
        .bind(item.id)
        .execute(&pool)
        .await
        .expect("deactivate");

    let err = process_status_transfer(
        &pool,
        &transfer_req(
            &tenant,
            item.id,
            wh,
            InvItemStatus::Available,
            InvItemStatus::Quarantine,
            5,
            "tx-inactive",
        ),
    )
    .await
    .expect_err("should fail");

    assert!(
        matches!(err, StatusTransferError::Guard(_)),
        "expected Guard error, got {:?}",
        err
    );
}

#[tokio::test]
#[serial]
async fn test_transfer_quarantine_to_damaged() {
    let pool = setup_db().await;
    let tenant = format!("st-t7-{}", Uuid::new_v4().simple());
    let wh = Uuid::new_v4();
    let item = ItemRepo::create(&pool, &create_item_req(&tenant, "ST-007"))
        .await
        .expect("create item");

    process_receipt(&pool, &receipt_req(&tenant, item.id, wh, 100, "r-7"), None)
        .await
        .expect("receipt");

    // First move some to quarantine
    process_status_transfer(
        &pool,
        &transfer_req(
            &tenant,
            item.id,
            wh,
            InvItemStatus::Available,
            InvItemStatus::Quarantine,
            40,
            "tx-7a",
        ),
    )
    .await
    .expect("av->qr");

    // Now move some from quarantine to damaged
    let (result, is_replay) = process_status_transfer(
        &pool,
        &transfer_req(
            &tenant,
            item.id,
            wh,
            InvItemStatus::Quarantine,
            InvItemStatus::Damaged,
            15,
            "tx-7b",
        ),
    )
    .await
    .expect("qr->dm");

    assert!(!is_replay);

    let buckets = read_buckets(&pool, &tenant, item.id, wh).await;
    assert_eq!(buckets["available"], 60);
    assert_eq!(buckets["quarantine"], 25);
    assert_eq!(buckets["damaged"], 15);

    // available_status_on_hand should only reflect the available bucket
    let avail_oh = read_available_on_hand(&pool, &tenant, item.id, wh).await;
    assert_eq!(
        avail_oh, 60,
        "available_status_on_hand must match available bucket"
    );

    let _ = result;
}

#[tokio::test]
#[serial]
async fn test_transfer_quarantine_back_to_available() {
    let pool = setup_db().await;
    let tenant = format!("st-t8-{}", Uuid::new_v4().simple());
    let wh = Uuid::new_v4();
    let item = ItemRepo::create(&pool, &create_item_req(&tenant, "ST-008"))
        .await
        .expect("create item");

    process_receipt(&pool, &receipt_req(&tenant, item.id, wh, 100, "r-8"), None)
        .await
        .expect("receipt");

    // Move 50 to quarantine
    process_status_transfer(
        &pool,
        &transfer_req(
            &tenant,
            item.id,
            wh,
            InvItemStatus::Available,
            InvItemStatus::Quarantine,
            50,
            "tx-8a",
        ),
    )
    .await
    .expect("av->qr");

    let avail_before = read_available_on_hand(&pool, &tenant, item.id, wh).await;
    assert_eq!(avail_before, 50);

    // Move 20 back from quarantine to available
    process_status_transfer(
        &pool,
        &transfer_req(
            &tenant,
            item.id,
            wh,
            InvItemStatus::Quarantine,
            InvItemStatus::Available,
            20,
            "tx-8b",
        ),
    )
    .await
    .expect("qr->av");

    let buckets = read_buckets(&pool, &tenant, item.id, wh).await;
    assert_eq!(buckets["available"], 70);
    assert_eq!(buckets["quarantine"], 30);

    let avail_oh = read_available_on_hand(&pool, &tenant, item.id, wh).await;
    assert_eq!(
        avail_oh, 70,
        "available_status_on_hand must increment when moving TO available"
    );
}

#[tokio::test]
#[serial]
async fn test_transfer_idempotency_conflict() {
    let pool = setup_db().await;
    let tenant = format!("st-t9-{}", Uuid::new_v4().simple());
    let wh = Uuid::new_v4();
    let item = ItemRepo::create(&pool, &create_item_req(&tenant, "ST-009"))
        .await
        .expect("create item");

    process_receipt(&pool, &receipt_req(&tenant, item.id, wh, 100, "r-9"), None)
        .await
        .expect("receipt");

    // First transfer with key "conflict-key"
    process_status_transfer(
        &pool,
        &transfer_req(
            &tenant,
            item.id,
            wh,
            InvItemStatus::Available,
            InvItemStatus::Quarantine,
            10,
            "conflict-key",
        ),
    )
    .await
    .expect("first transfer");

    // Second transfer with same key but different quantity (body differs)
    let err = process_status_transfer(
        &pool,
        &transfer_req(
            &tenant,
            item.id,
            wh,
            InvItemStatus::Available,
            InvItemStatus::Quarantine,
            99,
            "conflict-key",
        ),
    )
    .await
    .expect_err("should conflict");

    assert!(
        matches!(err, StatusTransferError::ConflictingIdempotencyKey),
        "expected ConflictingIdempotencyKey, got {:?}",
        err
    );
}
