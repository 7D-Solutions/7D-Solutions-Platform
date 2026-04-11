//! Integration tests: composite outbound ship endpoint (bd-6pyqw).
//!
//! Exercises OutboundShipService::execute against a real PostgreSQL database.
//! No mocks — QI integration mode controlled via QualityGateIntegration::permissive()
//! and QualityGateIntegration::holding().
//!
//! Scenarios:
//! 1. Happy path — no WO links, permissive gate → shipped
//! 2. Happy path — WO-linked lines, permissive gate → shipped (gate passes by mode)
//! 3. Quality gate blocks — WO-linked lines, AlwaysHold mode, no override → 403-equivalent error
//! 4. Override without permission — AlwaysHold, reason present, no perm → InsufficientPermissions
//! 5. Override with permission — AlwaysHold, reason present, has perm → shipped
//! 6. Not packed — shipment in confirmed state → NotPackable error
//! 7. Not outbound — inbound shipment → NotOutbound error

use chrono::Utc;
use serial_test::serial;
use shipping_receiving_rs::{
    domain::outbound_ship::{OutboundShipError, OutboundShipRequest, OutboundShipService},
    InventoryIntegration, QualityGateIntegration,
};
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

// ── DB helpers ───────────────────────────────────────────────

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

async fn insert_shipment(
    pool: &sqlx::PgPool,
    tenant_id: Uuid,
    direction: &str,
    status: &str,
) -> Uuid {
    let row: (Uuid,) = sqlx::query_as(
        "INSERT INTO shipments (tenant_id, direction, status) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(tenant_id)
    .bind(direction)
    .bind(status)
    .fetch_one(pool)
    .await
    .expect("insert shipment");
    row.0
}

/// Insert a shipment line, optionally with source WO link.
async fn insert_line(
    pool: &sqlx::PgPool,
    tenant_id: Uuid,
    shipment_id: Uuid,
    qty: i64,
    wo_id: Option<Uuid>,
) -> Uuid {
    let row: (Uuid,) = sqlx::query_as(
        r#"
        INSERT INTO shipment_lines
            (tenant_id, shipment_id, sku, qty_expected, qty_shipped, source_ref_type, source_ref_id)
        VALUES ($1, $2, 'COMP-SKU', $3, $3, $4, $5)
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(shipment_id)
    .bind(qty)
    .bind(wo_id.map(|_| "work_order"))
    .bind(wo_id)
    .fetch_one(pool)
    .await
    .expect("insert line");
    row.0
}

fn make_req(
    shipment_id: Uuid,
    tenant_id: Uuid,
    override_reason: Option<&str>,
    caller_can_override_qi: bool,
) -> OutboundShipRequest {
    OutboundShipRequest {
        shipment_id,
        tenant_id,
        shipped_at: Some(Utc::now()),
        override_reason: override_reason.map(|s| s.to_string()),
        caller_can_override_qi,
    }
}

async fn get_status(pool: &sqlx::PgPool, id: Uuid) -> String {
    let row: (String,) = sqlx::query_as("SELECT status FROM shipments WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("get status");
    row.0
}

// ── Tests ────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn happy_path_no_wo_links() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let inventory = InventoryIntegration::deterministic();
    let quality_gate = QualityGateIntegration::permissive();

    let ship_id = insert_shipment(&pool, tenant_id, "outbound", "packed").await;
    insert_line(&pool, tenant_id, ship_id, 5, None).await;

    let req = make_req(ship_id, tenant_id, None, false);
    let result = OutboundShipService::execute(&pool, req, &inventory, &quality_gate).await;
    assert!(result.is_ok(), "happy path failed: {:?}", result.err());

    assert_eq!(get_status(&pool, ship_id).await, "shipped");
}

#[tokio::test]
#[serial]
async fn happy_path_wo_links_permissive_gate() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let inventory = InventoryIntegration::deterministic();
    // Permissive mode always returns no holds — WO links don't matter
    let quality_gate = QualityGateIntegration::permissive();

    let ship_id = insert_shipment(&pool, tenant_id, "outbound", "packed").await;
    let wo_id = Uuid::new_v4();
    insert_line(&pool, tenant_id, ship_id, 3, Some(wo_id)).await;

    let req = make_req(ship_id, tenant_id, None, false);
    let result = OutboundShipService::execute(&pool, req, &inventory, &quality_gate).await;
    assert!(
        result.is_ok(),
        "permissive gate with WO links failed: {:?}",
        result.err()
    );

    assert_eq!(get_status(&pool, ship_id).await, "shipped");
}

#[tokio::test]
#[serial]
async fn quality_gate_blocks_when_held() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let inventory = InventoryIntegration::deterministic();
    let quality_gate = QualityGateIntegration::holding();

    let ship_id = insert_shipment(&pool, tenant_id, "outbound", "packed").await;
    let wo_id = Uuid::new_v4();
    insert_line(&pool, tenant_id, ship_id, 4, Some(wo_id)).await;

    let req = make_req(ship_id, tenant_id, None, false);
    let result = OutboundShipService::execute(&pool, req, &inventory, &quality_gate).await;

    assert!(result.is_err(), "hold mode must block shipment");
    match result.unwrap_err() {
        OutboundShipError::QualityGateHold { hold_count } => {
            assert_eq!(hold_count, 1, "expected 1 hold (one WO), got {hold_count}");
        }
        other => panic!("expected QualityGateHold, got: {other:?}"),
    }

    // Shipment must still be in packed — not shipped
    assert_eq!(get_status(&pool, ship_id).await, "packed");
}

#[tokio::test]
#[serial]
async fn override_without_permission_rejected() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let inventory = InventoryIntegration::deterministic();
    let quality_gate = QualityGateIntegration::holding();

    let ship_id = insert_shipment(&pool, tenant_id, "outbound", "packed").await;
    let wo_id = Uuid::new_v4();
    insert_line(&pool, tenant_id, ship_id, 2, Some(wo_id)).await;

    // Provides a reason but does NOT have quality_inspection.mutate permission
    let req = make_req(ship_id, tenant_id, Some("Urgent aerospace delivery"), false);
    let result = OutboundShipService::execute(&pool, req, &inventory, &quality_gate).await;

    assert!(result.is_err(), "override without permission must fail");
    match result.unwrap_err() {
        OutboundShipError::InsufficientPermissions => {}
        other => panic!("expected InsufficientPermissions, got: {other:?}"),
    }

    assert_eq!(get_status(&pool, ship_id).await, "packed");
}

#[tokio::test]
#[serial]
async fn override_with_permission_succeeds() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let inventory = InventoryIntegration::deterministic();
    let quality_gate = QualityGateIntegration::holding();

    let ship_id = insert_shipment(&pool, tenant_id, "outbound", "packed").await;
    let wo_id = Uuid::new_v4();
    insert_line(&pool, tenant_id, ship_id, 7, Some(wo_id)).await;

    // Has quality_inspection.mutate permission and supplies a reason
    let req = make_req(
        ship_id,
        tenant_id,
        Some("Customer authorized expedite, inspection waived per QE-42"),
        true,
    );
    let result = OutboundShipService::execute(&pool, req, &inventory, &quality_gate).await;
    assert!(
        result.is_ok(),
        "authorized override must succeed: {:?}",
        result.err()
    );

    assert_eq!(get_status(&pool, ship_id).await, "shipped");
}

#[tokio::test]
#[serial]
async fn not_packed_state_returns_not_packable() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let inventory = InventoryIntegration::deterministic();
    let quality_gate = QualityGateIntegration::permissive();

    // confirmed, not packed
    let ship_id = insert_shipment(&pool, tenant_id, "outbound", "confirmed").await;
    insert_line(&pool, tenant_id, ship_id, 3, None).await;

    let req = make_req(ship_id, tenant_id, None, false);
    let result = OutboundShipService::execute(&pool, req, &inventory, &quality_gate).await;

    assert!(result.is_err(), "non-packed state must fail");
    match result.unwrap_err() {
        OutboundShipError::NotPackable { current } => {
            assert_eq!(current, "confirmed");
        }
        other => panic!("expected NotPackable, got: {other:?}"),
    }
}

#[tokio::test]
#[serial]
async fn inbound_shipment_returns_not_outbound() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let inventory = InventoryIntegration::deterministic();
    let quality_gate = QualityGateIntegration::permissive();

    // Inbound shipments use "arrived" state as the closest to "packed"
    let ship_id = insert_shipment(&pool, tenant_id, "inbound", "arrived").await;
    insert_line(&pool, tenant_id, ship_id, 2, None).await;

    let req = make_req(ship_id, tenant_id, None, false);
    let result = OutboundShipService::execute(&pool, req, &inventory, &quality_gate).await;

    assert!(result.is_err(), "inbound shipment must fail");
    match result.unwrap_err() {
        OutboundShipError::NotOutbound => {}
        other => panic!("expected NotOutbound, got: {other:?}"),
    }
}

#[tokio::test]
#[serial]
async fn unknown_shipment_returns_not_found() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let inventory = InventoryIntegration::deterministic();
    let quality_gate = QualityGateIntegration::permissive();

    let req = make_req(Uuid::new_v4(), tenant_id, None, false);
    let result = OutboundShipService::execute(&pool, req, &inventory, &quality_gate).await;

    assert!(result.is_err(), "unknown shipment must fail");
    match result.unwrap_err() {
        OutboundShipError::NotFound => {}
        other => panic!("expected NotFound, got: {other:?}"),
    }
}
