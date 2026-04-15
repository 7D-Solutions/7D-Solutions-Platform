//! E2E integration tests: RMA receiving workflow (bd-2p78n).
//!
//! Tests run against a real PostgreSQL database on port 5454. Proves:
//! 1. RMA receive E2E — create receipt with items and condition notes
//! 2. Disposition workflow — inspect → quarantine → return_to_stock
//! 3. Invalid transition — scrap → return_to_stock rejected
//! 4. Tenant isolation — tenant_A data invisible to tenant_B
//! 5. Idempotency — same idempotency_key produces no duplicate
//! 6. Outbox event verification — each disposition change emits outbox event

use serial_test::serial;
use shipping_receiving_rs::domain::rma::{
    DispositionRequest, ReceiveRmaRequest, RmaItemInput, RmaService,
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

fn make_receive_request(
    rma_id: &str,
    customer_id: Uuid,
    idem_key: Option<&str>,
) -> ReceiveRmaRequest {
    ReceiveRmaRequest {
        rma_id: rma_id.to_string(),
        customer_id,
        condition_notes: Some("Damaged packaging, contents intact".to_string()),
        items: vec![
            RmaItemInput {
                sku: "PART-A100".to_string(),
                qty: 3,
                condition_notes: Some("Scratched surface".to_string()),
            },
            RmaItemInput {
                sku: "PART-B200".to_string(),
                qty: 1,
                condition_notes: None,
            },
        ],
        idempotency_key: idem_key.map(|s| s.to_string()),
    }
}

fn make_disposition(status: &str) -> DispositionRequest {
    DispositionRequest {
        disposition_status: status.to_string(),
    }
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

// ── 1. RMA receive E2E ─────────────────────────────────────

#[tokio::test]
#[serial]
async fn rma_receive_creates_receipt_with_items_and_condition_notes() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let customer_id = Uuid::new_v4();

    let req = make_receive_request("RMA-001", customer_id, None);
    let receipt = RmaService::receive(&pool, tenant_id, &req)
        .await
        .expect("receive RMA");

    // Verify receipt persisted correctly
    assert_eq!(receipt.tenant_id, tenant_id);
    assert_eq!(receipt.rma_id, "RMA-001");
    assert_eq!(receipt.customer_id, customer_id);
    assert_eq!(receipt.disposition_status, "received");
    assert_eq!(
        receipt.condition_notes.as_deref(),
        Some("Damaged packaging, contents intact")
    );

    // Verify items persisted
    let items = RmaService::list_items(&pool, receipt.id, tenant_id)
        .await
        .expect("list items");
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].sku, "PART-A100");
    assert_eq!(items[0].qty, 3);
    assert_eq!(
        items[0].condition_notes.as_deref(),
        Some("Scratched surface")
    );
    assert_eq!(items[1].sku, "PART-B200");
    assert_eq!(items[1].qty, 1);

    // Verify findable by ID
    let found = RmaService::find_by_id(&pool, receipt.id, tenant_id)
        .await
        .expect("find by id");
    assert!(found.is_some());
    assert_eq!(found.unwrap().rma_id, "RMA-001");
}

// ── 2. Disposition workflow ─────────────────────────────────

#[tokio::test]
#[serial]
async fn disposition_workflow_inspect_quarantine_return_to_stock() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let customer_id = Uuid::new_v4();

    // Create RMA receipt
    let req = make_receive_request("RMA-WF-001", customer_id, None);
    let receipt = RmaService::receive(&pool, tenant_id, &req)
        .await
        .expect("receive RMA");
    assert_eq!(receipt.disposition_status, "received");

    // received → inspect
    let updated = RmaService::transition_disposition(
        &pool,
        receipt.id,
        tenant_id,
        &make_disposition("inspect"),
    )
    .await
    .expect("transition to inspect");
    assert_eq!(updated.disposition_status, "inspect");

    // inspect → quarantine
    let updated = RmaService::transition_disposition(
        &pool,
        receipt.id,
        tenant_id,
        &make_disposition("quarantine"),
    )
    .await
    .expect("transition to quarantine");
    assert_eq!(updated.disposition_status, "quarantine");

    // quarantine → return_to_stock
    let updated = RmaService::transition_disposition(
        &pool,
        receipt.id,
        tenant_id,
        &make_disposition("return_to_stock"),
    )
    .await
    .expect("transition to return_to_stock");
    assert_eq!(updated.disposition_status, "return_to_stock");

    // Verify each transition persisted an outbox event
    let receipt_id_str = receipt.id.to_string();
    let disposition_events =
        count_outbox_events(&pool, &receipt_id_str, "sr.rma.disposition_changed").await;
    assert_eq!(
        disposition_events, 3,
        "expected 3 disposition_changed events (inspect, quarantine, return_to_stock)"
    );
}

// ── 3. Invalid transition ───────────────────────────────────

#[tokio::test]
#[serial]
async fn invalid_disposition_transition_rejected() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let customer_id = Uuid::new_v4();

    // Create and move to scrap (received → inspect → scrap)
    let req = make_receive_request("RMA-INV-001", customer_id, None);
    let receipt = RmaService::receive(&pool, tenant_id, &req)
        .await
        .expect("receive RMA");

    RmaService::transition_disposition(&pool, receipt.id, tenant_id, &make_disposition("inspect"))
        .await
        .expect("to inspect");

    RmaService::transition_disposition(&pool, receipt.id, tenant_id, &make_disposition("scrap"))
        .await
        .expect("to scrap");

    // Now attempt scrap → return_to_stock (invalid: scrap is terminal)
    let result = RmaService::transition_disposition(
        &pool,
        receipt.id,
        tenant_id,
        &make_disposition("return_to_stock"),
    )
    .await;

    assert!(result.is_err(), "scrap → return_to_stock must be rejected");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("terminal"),
        "error should mention terminal status: {err_msg}"
    );

    // Also test: received → quarantine (must go through inspect first)
    let req2 = make_receive_request("RMA-INV-002", customer_id, None);
    let receipt2 = RmaService::receive(&pool, tenant_id, &req2)
        .await
        .expect("receive second RMA");

    let result2 = RmaService::transition_disposition(
        &pool,
        receipt2.id,
        tenant_id,
        &make_disposition("quarantine"),
    )
    .await;

    assert!(
        result2.is_err(),
        "received → quarantine must be rejected (must go through inspect)"
    );
    let err_msg2 = result2.unwrap_err().to_string();
    assert!(
        err_msg2.contains("not allowed"),
        "error should mention not allowed: {err_msg2}"
    );
}

// ── 4. Tenant isolation ─────────────────────────────────────

#[tokio::test]
#[serial]
async fn tenant_isolation_rma_invisible_across_tenants() {
    let pool = setup_db().await;
    let tenant_a = Uuid::new_v4();
    let tenant_b = Uuid::new_v4();
    let customer_id = Uuid::new_v4();

    // Create RMA under tenant_A
    let req = make_receive_request("RMA-ISO-001", customer_id, None);
    let receipt = RmaService::receive(&pool, tenant_a, &req)
        .await
        .expect("receive under tenant_A");

    // Query as tenant_B — must return None
    let found = RmaService::find_by_id(&pool, receipt.id, tenant_b)
        .await
        .expect("find_by_id as tenant_B");
    assert!(
        found.is_none(),
        "tenant_B must not see tenant_A's RMA receipt"
    );

    // Items also invisible to tenant_B
    let items = RmaService::list_items(&pool, receipt.id, tenant_b)
        .await
        .expect("list_items as tenant_B");
    assert!(
        items.is_empty(),
        "tenant_B must not see tenant_A's RMA items"
    );

    // Disposition transition as tenant_B must fail (not found)
    let result = RmaService::transition_disposition(
        &pool,
        receipt.id,
        tenant_b,
        &make_disposition("inspect"),
    )
    .await;
    assert!(
        result.is_err(),
        "tenant_B must not be able to transition tenant_A's RMA"
    );
}

// ── 5. Idempotency ──────────────────────────────────────────

#[tokio::test]
#[serial]
async fn idempotent_rma_receive_no_duplicate() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let customer_id = Uuid::new_v4();
    let idem_key = "idem-rma-001";

    // First receive
    let req = make_receive_request("RMA-IDEM-001", customer_id, Some(idem_key));
    let receipt1 = RmaService::receive(&pool, tenant_id, &req)
        .await
        .expect("first receive");

    // Second receive with same idempotency key
    let req2 = make_receive_request("RMA-IDEM-001", customer_id, Some(idem_key));
    let receipt2 = RmaService::receive(&pool, tenant_id, &req2)
        .await
        .expect("second receive (idempotent)");

    // Must return the same receipt, not a new one
    assert_eq!(
        receipt1.id, receipt2.id,
        "idempotent receive must return same receipt"
    );

    // Verify only one receipt exists with this rma_id for this tenant
    let count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM rma_receipts WHERE tenant_id = $1 AND rma_id = $2")
            .bind(tenant_id)
            .bind("RMA-IDEM-001")
            .fetch_one(&pool)
            .await
            .expect("count receipts");
    assert_eq!(count.0, 1, "must have exactly one receipt, not a duplicate");
}

// ── 6. Outbox event verification ────────────────────────────

#[tokio::test]
#[serial]
async fn outbox_events_emitted_for_each_disposition_change() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let customer_id = Uuid::new_v4();

    // Create RMA receipt
    let req = make_receive_request("RMA-EVT-001", customer_id, None);
    let receipt = RmaService::receive(&pool, tenant_id, &req)
        .await
        .expect("receive RMA");
    let receipt_id_str = receipt.id.to_string();

    // Verify receive event
    let received_events = count_outbox_events(&pool, &receipt_id_str, "sr.rma.received").await;
    assert_eq!(
        received_events, 1,
        "must have exactly 1 rma.received outbox event"
    );

    // Transition: received → inspect
    RmaService::transition_disposition(&pool, receipt.id, tenant_id, &make_disposition("inspect"))
        .await
        .expect("to inspect");

    let disp_events =
        count_outbox_events(&pool, &receipt_id_str, "sr.rma.disposition_changed").await;
    assert_eq!(disp_events, 1, "1 disposition_changed after inspect");

    // Transition: inspect → quarantine
    RmaService::transition_disposition(
        &pool,
        receipt.id,
        tenant_id,
        &make_disposition("quarantine"),
    )
    .await
    .expect("to quarantine");

    let disp_events =
        count_outbox_events(&pool, &receipt_id_str, "sr.rma.disposition_changed").await;
    assert_eq!(disp_events, 2, "2 disposition_changed after quarantine");

    // Transition: quarantine → scrap
    RmaService::transition_disposition(&pool, receipt.id, tenant_id, &make_disposition("scrap"))
        .await
        .expect("to scrap");

    let disp_events =
        count_outbox_events(&pool, &receipt_id_str, "sr.rma.disposition_changed").await;
    assert_eq!(disp_events, 3, "3 disposition_changed after scrap");

    // Verify each event has correct tenant_id in payload
    let events: Vec<(serde_json::Value,)> = sqlx::query_as(
        r#"
        SELECT payload FROM sr_events_outbox
        WHERE aggregate_id = $1 AND event_type = 'sr.rma.disposition_changed'
        ORDER BY created_at
        "#,
    )
    .bind(&receipt_id_str)
    .fetch_all(&pool)
    .await
    .expect("fetch outbox events");

    assert_eq!(events.len(), 3);

    // First event: received → inspect
    let p0 = &events[0].0;
    assert_eq!(p0["from_disposition"], "received");
    assert_eq!(p0["to_disposition"], "inspect");
    assert_eq!(p0["tenant_id"], tenant_id.to_string());

    // Second event: inspect → quarantine
    let p1 = &events[1].0;
    assert_eq!(p1["from_disposition"], "inspect");
    assert_eq!(p1["to_disposition"], "quarantine");

    // Third event: quarantine → scrap
    let p2 = &events[2].0;
    assert_eq!(p2["from_disposition"], "quarantine");
    assert_eq!(p2["to_disposition"], "scrap");
}
