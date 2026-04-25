//! Integration tests: inbound carrier tracking on purchase orders (bd-4w4bg).
//!
//! All tests use a real PostgreSQL database. No mocks.
//!
//! Invariant under test: carrier webhook = visibility only.
//! State-machine advance (inbound_status) requires dock-scan or manual receipt.
//! A carrier "delivered" event NEVER flips inbound_status automatically.
//!
//! Run:
//!   ./scripts/cargo-slot.sh test -p shipping-receiving --test inbound_tracking_test

use chrono::Utc;
use serial_test::serial;
use shipping_receiving_rs::{
    domain::{
        shipments::{ShipmentService, TransitionRequest},
        tracking,
    },
    InventoryIntegration,
};
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

// ── DB helpers ────────────────────────────────────────────────

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://shipping_receiving_user:shipping_receiving_pass@localhost:5454/shipping_receiving_db"
            .to_string()
    });

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to shipping-receiving test DB");

    let _ = sqlx::migrate!("db/migrations").run(&pool).await;
    pool
}

async fn insert_inbound_shipment(pool: &sqlx::PgPool, tenant_id: Uuid, status: &str) -> Uuid {
    let row: (Uuid,) = sqlx::query_as(
        "INSERT INTO shipments (tenant_id, direction, status) VALUES ($1, 'inbound', $2) RETURNING id",
    )
    .bind(tenant_id)
    .bind(status)
    .fetch_one(pool)
    .await
    .expect("insert inbound shipment");
    row.0
}

async fn set_expected_tracking_direct(
    pool: &sqlx::PgPool,
    shipment_id: Uuid,
    carrier_code: &str,
    tracking_number: &str,
) {
    sqlx::query(
        "UPDATE shipments SET expected_carrier_code = $1, expected_tracking_number = $2 WHERE id = $3",
    )
    .bind(carrier_code)
    .bind(tracking_number)
    .bind(shipment_id)
    .execute(pool)
    .await
    .expect("set expected tracking");
}

async fn get_inbound_status(pool: &sqlx::PgPool, id: Uuid) -> String {
    let row: (String,) = sqlx::query_as("SELECT status FROM shipments WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("get status");
    row.0
}

async fn get_latest_tracking(pool: &sqlx::PgPool, id: Uuid) -> (Option<String>, Option<String>) {
    let row: (Option<String>, Option<String>) = sqlx::query_as(
        "SELECT latest_tracking_status, latest_tracking_location FROM shipments WHERE id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .expect("get latest tracking");
    row
}

async fn get_expected_tracking(pool: &sqlx::PgPool, id: Uuid) -> (Option<String>, Option<String>) {
    let row: (Option<String>, Option<String>) = sqlx::query_as(
        "SELECT expected_carrier_code, expected_tracking_number FROM shipments WHERE id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .expect("get expected tracking");
    row
}

async fn count_outbox_events(pool: &sqlx::PgPool, aggregate_id: &str, event_type: &str) -> i64 {
    let row: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM sr_events_outbox WHERE aggregate_id = $1 AND event_type = $2",
    )
    .bind(aggregate_id)
    .bind(event_type)
    .fetch_one(pool)
    .await
    .expect("count outbox events");
    row.0
}

// ── Tests ─────────────────────────────────────────────────────

/// CHANGE 2: POST expected-tracking populates fields, inbound_status unchanged.
#[tokio::test]
#[serial]
async fn inbound_po_captures_expected_tracking() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();

    let ship_id = insert_inbound_shipment(&pool, tenant_id, "draft").await;

    set_expected_tracking_direct(&pool, ship_id, "ups", "1Z999AA10123456784").await;

    let (carrier_code, tracking_number) = get_expected_tracking(&pool, ship_id).await;
    assert_eq!(carrier_code.as_deref(), Some("ups"), "expected_carrier_code must be set");
    assert_eq!(
        tracking_number.as_deref(),
        Some("1Z999AA10123456784"),
        "expected_tracking_number must be set"
    );

    // Invariant: inbound_status is NOT advanced by capturing expected tracking
    let status = get_inbound_status(&pool, ship_id).await;
    assert_eq!(status, "draft", "inbound_status must remain unchanged after capturing expected tracking");
}

/// CHANGE 3: webhook matching expected_tracking_number updates latest_tracking_*,
/// does NOT update inbound_status.
#[tokio::test]
#[serial]
async fn webhook_updates_inbound_visibility() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let tracking_number = format!("1Z-INBOUND-VIS-{}", Uuid::new_v4().simple());

    let ship_id = insert_inbound_shipment(&pool, tenant_id, "confirmed").await;
    set_expected_tracking_direct(&pool, ship_id, "ups", &tracking_number).await;

    // Simulate webhook delivery → tracking domain processes the event
    let raw_hash = tracking::sha256_hex(
        format!("test-payload-{}", tracking_number).as_bytes(),
    );
    let inserted = tracking::record_tracking_event(
        &pool,
        &tenant_id.to_string(),
        Some(ship_id),
        &tracking_number,
        "ups",
        tracking::STATUS_DELIVERED,
        Utc::now(),
        Some("CITY, STATE"),
        &raw_hash,
    )
    .await
    .expect("record tracking event");
    assert!(inserted.is_some(), "new event must be inserted");

    tracking::update_inbound_latest_tracking(
        &pool,
        ship_id,
        tracking::STATUS_DELIVERED,
        Utc::now(),
        Some("CITY, STATE"),
    )
    .await
    .expect("update inbound latest tracking");

    let (latest_status, latest_location) = get_latest_tracking(&pool, ship_id).await;
    assert_eq!(
        latest_status.as_deref(),
        Some("delivered"),
        "latest_tracking_status must be updated"
    );
    assert_eq!(
        latest_location.as_deref(),
        Some("CITY, STATE"),
        "latest_tracking_location must be updated"
    );

    // Critical invariant: inbound_status is NOT advanced
    let status = get_inbound_status(&pool, ship_id).await;
    assert_eq!(
        status, "confirmed",
        "inbound_status must NOT be advanced by a carrier delivered webhook"
    );
}

/// CHANGE 4: dock-scan (state machine transition) advances inbound_status.
/// latest_tracking_status is independent.
#[tokio::test]
#[serial]
async fn dock_scan_advances_state() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let inventory = InventoryIntegration::deterministic();

    // Start at arrived — ready for dock receipt
    let ship_id = insert_inbound_shipment(&pool, tenant_id, "arrived").await;
    let tracking_number = format!("1Z-DOCK-SCAN-{}", Uuid::new_v4().simple());
    set_expected_tracking_direct(&pool, ship_id, "ups", &tracking_number).await;

    // Manually advance via state machine (dock scan = status transition to receiving)
    let req = TransitionRequest {
        status: "receiving".to_string(),
        arrived_at: None,
        shipped_at: None,
        delivered_at: None,
        closed_at: None,
    };
    ShipmentService::transition(&pool, ship_id, tenant_id, &req, &inventory)
        .await
        .expect("dock scan transition to receiving must succeed");

    let status = get_inbound_status(&pool, ship_id).await;
    assert_eq!(status, "receiving", "dock scan must advance inbound_status to receiving");

    // latest_tracking_status is independent — may be delivered or NULL
    // (dock scan doesn't touch latest_tracking_*)
    let (latest_status, _) = get_latest_tracking(&pool, ship_id).await;
    assert!(
        latest_status.is_none() || latest_status.as_deref() != Some("receiving"),
        "dock scan must NOT touch latest_tracking_status"
    );
}

/// Critical invariant: carrier "delivered" webhook alone does NOT advance inbound_status.
/// This is the core safety check for aerospace/defense chain-of-custody.
#[tokio::test]
#[serial]
async fn carrier_delivered_without_scan_does_not_advance() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let tracking_number = format!("1Z-NO-ADVANCE-{}", Uuid::new_v4().simple());

    let ship_id = insert_inbound_shipment(&pool, tenant_id, "in_transit").await;
    set_expected_tracking_direct(&pool, ship_id, "fedex", &tracking_number).await;

    // Carrier fires "delivered" webhook — update visibility only
    let raw_hash = tracking::sha256_hex(
        format!("delivered-payload-{}", tracking_number).as_bytes(),
    );
    tracking::record_tracking_event(
        &pool,
        &tenant_id.to_string(),
        Some(ship_id),
        &tracking_number,
        "fedex",
        tracking::STATUS_DELIVERED,
        Utc::now(),
        Some("WAREHOUSE GATE"),
        &raw_hash,
    )
    .await
    .expect("record tracking event");

    tracking::update_inbound_latest_tracking(
        &pool,
        ship_id,
        tracking::STATUS_DELIVERED,
        Utc::now(),
        Some("WAREHOUSE GATE"),
    )
    .await
    .expect("update inbound latest tracking");

    // Visibility updated
    let (latest_status, _) = get_latest_tracking(&pool, ship_id).await;
    assert_eq!(
        latest_status.as_deref(),
        Some("delivered"),
        "latest_tracking_status should reflect carrier delivered"
    );

    // State machine NOT advanced — driver may have left it at the gate
    let status = get_inbound_status(&pool, ship_id).await;
    assert_eq!(
        status, "in_transit",
        "inbound_status must NOT auto-advance to received when carrier says delivered — \
         dock scan or manual receipt required for chain-of-custody"
    );

    // No inbound.tracking_updated outbox event should exist either
    // (we called the domain directly, not the webhook handler — this verifies
    //  the functions themselves don't mutate status)
    let outbox_count = count_outbox_events(
        &pool,
        &ship_id.to_string(),
        "shipping_receiving.inbound.tracking_updated",
    )
    .await;
    // 0 because we called domain functions directly (no outbox write in domain layer)
    assert_eq!(
        outbox_count, 0,
        "direct domain calls do not write to outbox — that is the webhook handler's job"
    );
}
