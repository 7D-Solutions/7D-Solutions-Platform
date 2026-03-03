//! E2E integration tests: Inspection routing at receiving (bd-x6ok5).
//!
//! Tests run against a real PostgreSQL database. Proves:
//! 1. Route line to inspection — happy path
//! 2. Route line to stock — happy path
//! 3. Idempotent routing: same idempotency_key returns existing record
//! 4. Already-routed line rejects second routing attempt
//! 5. Guard: rejects outbound shipments
//! 6. Guard: rejects wrong shipment status (not receiving)
//! 7. Guard: rejects nonexistent line
//! 8. Tenant isolation: cannot route another tenant's lines
//! 9. Outbox event emitted for each routing decision

use serial_test::serial;
use shipping_receiving_rs::domain::inspection_routing::{
    InspectionRoutingService, RouteLineRequest,
};
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

// ── Test DB helpers ─────────────────────────────────────────

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| "postgres://shipping_receiving_user:shipping_receiving_pass@localhost:5454/shipping_receiving_db".to_string());

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

async fn insert_line(
    pool: &sqlx::PgPool,
    tenant_id: Uuid,
    shipment_id: Uuid,
    qty_expected: i64,
) -> Uuid {
    let row: (Uuid,) = sqlx::query_as(
        r#"
        INSERT INTO shipment_lines (tenant_id, shipment_id, sku, qty_expected)
        VALUES ($1, $2, 'TEST-SKU', $3)
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(shipment_id)
    .bind(qty_expected)
    .fetch_one(pool)
    .await
    .expect("insert line");
    row.0
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

async fn count_routings_for_line(pool: &sqlx::PgPool, tenant_id: Uuid, line_id: Uuid) -> i64 {
    let row: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM inspection_routings WHERE tenant_id = $1 AND shipment_line_id = $2",
    )
    .bind(tenant_id)
    .bind(line_id)
    .fetch_one(pool)
    .await
    .expect("count routings for line");
    row.0
}

fn make_route_req(decision: &str, reason: Option<&str>, idem_key: Option<&str>) -> RouteLineRequest {
    RouteLineRequest {
        route_decision: decision.to_string(),
        reason: reason.map(String::from),
        idempotency_key: idem_key.map(String::from),
    }
}

// ── Tests ───────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn route_line_to_inspection_happy_path() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();

    let ship_id = insert_shipment(&pool, tenant_id, "inbound", "receiving").await;
    let line_id = insert_line(&pool, tenant_id, ship_id, 10).await;

    let req = make_route_req("send_to_inspection", Some("visual defect suspected"), None);
    let result =
        InspectionRoutingService::route_line(&pool, ship_id, line_id, tenant_id, None, &req).await;

    assert!(result.is_ok(), "routing failed: {:?}", result.err());
    let routing = result.unwrap();
    assert_eq!(routing.route_decision, "send_to_inspection");
    assert_eq!(routing.reason.as_deref(), Some("visual defect suspected"));
    assert_eq!(routing.shipment_id, ship_id);
    assert_eq!(routing.shipment_line_id, line_id);
    assert_eq!(routing.tenant_id, tenant_id);
}

#[tokio::test]
#[serial]
async fn route_line_to_stock_happy_path() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();

    let ship_id = insert_shipment(&pool, tenant_id, "inbound", "receiving").await;
    let line_id = insert_line(&pool, tenant_id, ship_id, 5).await;

    let req = make_route_req("direct_to_stock", None, None);
    let result =
        InspectionRoutingService::route_line(&pool, ship_id, line_id, tenant_id, None, &req).await;

    assert!(result.is_ok(), "routing failed: {:?}", result.err());
    let routing = result.unwrap();
    assert_eq!(routing.route_decision, "direct_to_stock");
    assert!(routing.reason.is_none());
}

#[tokio::test]
#[serial]
async fn route_line_emits_outbox_event() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();

    let ship_id = insert_shipment(&pool, tenant_id, "inbound", "receiving").await;
    let line_id = insert_line(&pool, tenant_id, ship_id, 10).await;

    let req = make_route_req("send_to_inspection", None, None);
    let routing =
        InspectionRoutingService::route_line(&pool, ship_id, line_id, tenant_id, None, &req)
            .await
            .expect("routing");

    let routing_id_str = routing.id.to_string();
    let count = count_outbox_events(
        &pool,
        &routing_id_str,
        "sr.receipt_routed_to_inspection.v1",
    )
    .await;
    assert_eq!(count, 1, "expected exactly 1 outbox event for routing");
}

#[tokio::test]
#[serial]
async fn route_to_stock_emits_stock_event() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();

    let ship_id = insert_shipment(&pool, tenant_id, "inbound", "receiving").await;
    let line_id = insert_line(&pool, tenant_id, ship_id, 10).await;

    let req = make_route_req("direct_to_stock", None, None);
    let routing =
        InspectionRoutingService::route_line(&pool, ship_id, line_id, tenant_id, None, &req)
            .await
            .expect("routing");

    let routing_id_str = routing.id.to_string();
    let count =
        count_outbox_events(&pool, &routing_id_str, "sr.receipt_routed_to_stock.v1").await;
    assert_eq!(count, 1, "expected exactly 1 stock routing outbox event");
}

#[tokio::test]
#[serial]
async fn idempotent_routing_same_key_returns_existing() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();

    let ship_id = insert_shipment(&pool, tenant_id, "inbound", "receiving").await;
    let line_id = insert_line(&pool, tenant_id, ship_id, 10).await;

    let idem_key = Uuid::new_v4().to_string();

    let req1 = make_route_req("send_to_inspection", Some("first"), Some(&idem_key));
    let routing1 =
        InspectionRoutingService::route_line(&pool, ship_id, line_id, tenant_id, None, &req1)
            .await
            .expect("first routing");

    // Second call with same key — should return same routing, not create a new one
    let req2 = make_route_req("send_to_inspection", Some("second"), Some(&idem_key));
    let routing2 =
        InspectionRoutingService::route_line(&pool, ship_id, line_id, tenant_id, None, &req2)
            .await
            .expect("second routing (idempotent)");

    assert_eq!(routing1.id, routing2.id, "must return same routing record");
    assert_eq!(
        routing2.reason.as_deref(),
        Some("first"),
        "reason must be from the first call"
    );

    // Only 1 record in the DB
    let count = count_routings_for_line(&pool, tenant_id, line_id).await;
    assert_eq!(count, 1, "must have exactly 1 routing record");
}

#[tokio::test]
#[serial]
async fn already_routed_line_rejects_second_routing() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();

    let ship_id = insert_shipment(&pool, tenant_id, "inbound", "receiving").await;
    let line_id = insert_line(&pool, tenant_id, ship_id, 10).await;

    let req1 = make_route_req("direct_to_stock", None, None);
    InspectionRoutingService::route_line(&pool, ship_id, line_id, tenant_id, None, &req1)
        .await
        .expect("first routing");

    // Second attempt with different decision and no idempotency key
    let req2 = make_route_req("send_to_inspection", None, None);
    let result =
        InspectionRoutingService::route_line(&pool, ship_id, line_id, tenant_id, None, &req2)
            .await;

    assert!(result.is_err(), "must reject re-routing");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("already routed"),
        "error must mention already routed: {err_msg}"
    );
}

#[tokio::test]
#[serial]
async fn rejects_outbound_shipment() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();

    let ship_id = insert_shipment(&pool, tenant_id, "outbound", "picking").await;
    let line_id = insert_line(&pool, tenant_id, ship_id, 10).await;

    let req = make_route_req("direct_to_stock", None, None);
    let result =
        InspectionRoutingService::route_line(&pool, ship_id, line_id, tenant_id, None, &req).await;

    assert!(result.is_err(), "must reject outbound shipment");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("inbound"),
        "error must mention inbound: {err_msg}"
    );
}

#[tokio::test]
#[serial]
async fn rejects_wrong_status_not_receiving() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();

    // Try with "arrived" status (not yet receiving)
    let ship_id = insert_shipment(&pool, tenant_id, "inbound", "arrived").await;
    let line_id = insert_line(&pool, tenant_id, ship_id, 10).await;

    let req = make_route_req("direct_to_stock", None, None);
    let result =
        InspectionRoutingService::route_line(&pool, ship_id, line_id, tenant_id, None, &req).await;

    assert!(result.is_err(), "must reject non-receiving status");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("receiving"),
        "error must mention receiving: {err_msg}"
    );
}

#[tokio::test]
#[serial]
async fn rejects_nonexistent_shipment() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let fake_ship = Uuid::new_v4();
    let fake_line = Uuid::new_v4();

    let req = make_route_req("direct_to_stock", None, None);
    let result =
        InspectionRoutingService::route_line(&pool, fake_ship, fake_line, tenant_id, None, &req)
            .await;

    assert!(result.is_err(), "must reject nonexistent shipment");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("not found"),
        "error must mention not found: {err_msg}"
    );
}

#[tokio::test]
#[serial]
async fn rejects_nonexistent_line() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();

    let ship_id = insert_shipment(&pool, tenant_id, "inbound", "receiving").await;
    let fake_line = Uuid::new_v4();

    let req = make_route_req("direct_to_stock", None, None);
    let result =
        InspectionRoutingService::route_line(&pool, ship_id, fake_line, tenant_id, None, &req)
            .await;

    assert!(result.is_err(), "must reject nonexistent line");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("not found"),
        "error must mention not found: {err_msg}"
    );
}

#[tokio::test]
#[serial]
async fn tenant_isolation_cannot_route_other_tenants_line() {
    let pool = setup_db().await;
    let tenant_a = Uuid::new_v4();
    let tenant_b = Uuid::new_v4();

    let ship_id = insert_shipment(&pool, tenant_a, "inbound", "receiving").await;
    let line_id = insert_line(&pool, tenant_a, ship_id, 10).await;

    // Tenant B tries to route tenant A's line
    let req = make_route_req("direct_to_stock", None, None);
    let result =
        InspectionRoutingService::route_line(&pool, ship_id, line_id, tenant_b, None, &req).await;

    assert!(
        result.is_err(),
        "must reject routing across tenant boundaries"
    );
}

#[tokio::test]
#[serial]
async fn rejects_invalid_route_decision() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();

    let ship_id = insert_shipment(&pool, tenant_id, "inbound", "receiving").await;
    let line_id = insert_line(&pool, tenant_id, ship_id, 10).await;

    let req = make_route_req("send_to_mars", None, None);
    let result =
        InspectionRoutingService::route_line(&pool, ship_id, line_id, tenant_id, None, &req).await;

    assert!(result.is_err(), "must reject invalid route decision");
}

#[tokio::test]
#[serial]
async fn list_routings_for_shipment() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();

    let ship_id = insert_shipment(&pool, tenant_id, "inbound", "receiving").await;
    let line1 = insert_line(&pool, tenant_id, ship_id, 10).await;
    let line2 = insert_line(&pool, tenant_id, ship_id, 5).await;

    let req1 = make_route_req("send_to_inspection", Some("damaged packaging"), None);
    InspectionRoutingService::route_line(&pool, ship_id, line1, tenant_id, None, &req1)
        .await
        .expect("route line 1");

    let req2 = make_route_req("direct_to_stock", None, None);
    InspectionRoutingService::route_line(&pool, ship_id, line2, tenant_id, None, &req2)
        .await
        .expect("route line 2");

    let routings =
        InspectionRoutingService::list_for_shipment(&pool, ship_id, tenant_id)
            .await
            .expect("list routings");

    assert_eq!(routings.len(), 2, "must have 2 routings");

    let decisions: Vec<&str> = routings.iter().map(|r| r.route_decision.as_str()).collect();
    assert!(decisions.contains(&"send_to_inspection"));
    assert!(decisions.contains(&"direct_to_stock"));
}

#[tokio::test]
#[serial]
async fn routed_by_actor_is_recorded() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let actor_id = Uuid::new_v4();

    let ship_id = insert_shipment(&pool, tenant_id, "inbound", "receiving").await;
    let line_id = insert_line(&pool, tenant_id, ship_id, 10).await;

    let req = make_route_req("direct_to_stock", None, None);
    let routing = InspectionRoutingService::route_line(
        &pool,
        ship_id,
        line_id,
        tenant_id,
        Some(actor_id),
        &req,
    )
    .await
    .expect("routing");

    assert_eq!(routing.routed_by, Some(actor_id));
}
