//! E2E integration tests: Shipping document generation hooks (bd-12j00).
//!
//! Tests run against a real PostgreSQL database on port 5454. Proves:
//! 1. Request lifecycle E2E — create doc request, verify persistence
//! 2. Tenant isolation — tenant_A data invisible to tenant_B
//! 3. Idempotency — same idempotency_key produces no duplicate
//! 4. Outbox event — creation emits outbox event with correct type and tenant_id
//! 5. Status transitions — each transition emits an outbox event

use serial_test::serial;
use shipping_receiving_rs::domain::shipping_docs::{
    CreateDocRequest, ShippingDocService, TransitionStatusRequest,
};
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

// ── Test DB helpers ─────────────────────────────────────────

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for integration tests");

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to shipping-receiving test DB");

    let _ = sqlx::migrate!("db/migrations").run(&pool).await;
    pool
}

fn make_doc_request(
    shipment_id: Uuid,
    doc_type: &str,
    idem_key: Option<&str>,
) -> CreateDocRequest {
    CreateDocRequest {
        shipment_id,
        doc_type: doc_type.to_string(),
        payload_ref: Some("s3://docs/metadata.json".to_string()),
        idempotency_key: idem_key.map(|s| s.to_string()),
    }
}

fn make_transition(status: &str) -> TransitionStatusRequest {
    TransitionStatusRequest {
        status: status.to_string(),
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

// ── 1. Request lifecycle E2E ─────────────────────────────────

#[tokio::test]
#[serial]
async fn create_shipping_doc_request_persists_with_correct_status_and_shipment_ref() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let shipment_id = Uuid::new_v4();

    let req = make_doc_request(shipment_id, "packing_slip", None);
    let doc_req = ShippingDocService::create(&pool, tenant_id, &req)
        .await
        .expect("create doc request");

    // Verify persisted correctly
    assert_eq!(doc_req.tenant_id, tenant_id);
    assert_eq!(doc_req.shipment_id, shipment_id);
    assert_eq!(doc_req.doc_type, "packing_slip");
    assert_eq!(doc_req.status, "requested");
    assert_eq!(doc_req.payload_ref.as_deref(), Some("s3://docs/metadata.json"));

    // Verify findable by ID
    let found = ShippingDocService::find_by_id(&pool, doc_req.id, tenant_id)
        .await
        .expect("find by id");
    assert!(found.is_some());
    let found = found.unwrap();
    assert_eq!(found.shipment_id, shipment_id);
    assert_eq!(found.doc_type, "packing_slip");

    // Verify listable by shipment
    let list = ShippingDocService::list_by_shipment(&pool, shipment_id, tenant_id)
        .await
        .expect("list by shipment");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, doc_req.id);

    // Also test bill_of_lading doc type
    let req2 = make_doc_request(shipment_id, "bill_of_lading", None);
    let doc_req2 = ShippingDocService::create(&pool, tenant_id, &req2)
        .await
        .expect("create BOL request");
    assert_eq!(doc_req2.doc_type, "bill_of_lading");

    // Should now have 2 docs for this shipment
    let list2 = ShippingDocService::list_by_shipment(&pool, shipment_id, tenant_id)
        .await
        .expect("list by shipment (2)");
    assert_eq!(list2.len(), 2);
}

// ── 2. Tenant isolation ──────────────────────────────────────

#[tokio::test]
#[serial]
async fn tenant_isolation_doc_request_invisible_across_tenants() {
    let pool = setup_db().await;
    let tenant_a = Uuid::new_v4();
    let tenant_b = Uuid::new_v4();
    let shipment_id = Uuid::new_v4();

    // Create doc request under tenant_A
    let req = make_doc_request(shipment_id, "packing_slip", None);
    let doc_req = ShippingDocService::create(&pool, tenant_a, &req)
        .await
        .expect("create under tenant_A");

    // Query as tenant_B — must return None
    let found = ShippingDocService::find_by_id(&pool, doc_req.id, tenant_b)
        .await
        .expect("find_by_id as tenant_B");
    assert!(
        found.is_none(),
        "tenant_B must not see tenant_A's doc request"
    );

    // List by shipment as tenant_B — must return empty
    let list = ShippingDocService::list_by_shipment(&pool, shipment_id, tenant_b)
        .await
        .expect("list as tenant_B");
    assert!(
        list.is_empty(),
        "tenant_B must not see tenant_A's doc requests"
    );

    // Status transition as tenant_B must fail (not found)
    let result = ShippingDocService::transition_status(
        &pool,
        doc_req.id,
        tenant_b,
        &make_transition("generating"),
    )
    .await;
    assert!(
        result.is_err(),
        "tenant_B must not be able to transition tenant_A's doc request"
    );
}

// ── 3. Idempotency ───────────────────────────────────────────

#[tokio::test]
#[serial]
async fn idempotent_doc_request_no_duplicate() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let shipment_id = Uuid::new_v4();
    let idem_key = "idem-doc-001";

    // First create
    let req = make_doc_request(shipment_id, "packing_slip", Some(idem_key));
    let doc1 = ShippingDocService::create(&pool, tenant_id, &req)
        .await
        .expect("first create");

    // Second create with same idempotency key
    let req2 = make_doc_request(shipment_id, "packing_slip", Some(idem_key));
    let doc2 = ShippingDocService::create(&pool, tenant_id, &req2)
        .await
        .expect("second create (idempotent)");

    // Must return the same request, not a new one
    assert_eq!(
        doc1.id, doc2.id,
        "idempotent create must return same doc request"
    );

    // Verify only one request exists for this shipment/tenant
    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM sr_shipping_doc_requests WHERE tenant_id = $1 AND shipment_id = $2 AND idempotency_key = $3",
    )
    .bind(tenant_id)
    .bind(shipment_id)
    .bind(idem_key)
    .fetch_one(&pool)
    .await
    .expect("count requests");
    assert_eq!(count.0, 1, "must have exactly one request, not a duplicate");
}

// ── 4. Outbox event ──────────────────────────────────────────

#[tokio::test]
#[serial]
async fn outbox_event_emitted_on_doc_request_creation() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let shipment_id = Uuid::new_v4();

    let req = make_doc_request(shipment_id, "packing_slip", None);
    let doc_req = ShippingDocService::create(&pool, tenant_id, &req)
        .await
        .expect("create doc request");

    let doc_req_id_str = doc_req.id.to_string();

    // Verify outbox event exists
    let event_count =
        count_outbox_events(&pool, &doc_req_id_str, "sr.shipping_doc.requested").await;
    assert_eq!(
        event_count, 1,
        "must have exactly 1 doc.requested outbox event"
    );

    // Verify event has correct tenant_id and type in payload
    let events: Vec<(serde_json::Value, String)> = sqlx::query_as(
        r#"
        SELECT payload, tenant_id FROM sr_events_outbox
        WHERE aggregate_id = $1 AND event_type = 'sr.shipping_doc.requested'
        "#,
    )
    .bind(&doc_req_id_str)
    .fetch_all(&pool)
    .await
    .expect("fetch outbox events");

    assert_eq!(events.len(), 1);
    let (payload, outbox_tenant_id) = &events[0];
    assert_eq!(outbox_tenant_id, &tenant_id.to_string());
    assert_eq!(payload["doc_type"], "packing_slip");
    assert_eq!(payload["tenant_id"], tenant_id.to_string());
    assert_eq!(payload["shipment_id"], shipment_id.to_string());
}

// ── 5. Status transitions ────────────────────────────────────

#[tokio::test]
#[serial]
async fn status_transitions_emit_outbox_events() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let shipment_id = Uuid::new_v4();

    // Create doc request
    let req = make_doc_request(shipment_id, "bill_of_lading", None);
    let doc_req = ShippingDocService::create(&pool, tenant_id, &req)
        .await
        .expect("create doc request");
    let doc_req_id_str = doc_req.id.to_string();
    assert_eq!(doc_req.status, "requested");

    // requested → generating
    let updated = ShippingDocService::transition_status(
        &pool,
        doc_req.id,
        tenant_id,
        &make_transition("generating"),
    )
    .await
    .expect("transition to generating");
    assert_eq!(updated.status, "generating");

    let status_events =
        count_outbox_events(&pool, &doc_req_id_str, "sr.shipping_doc.status_changed").await;
    assert_eq!(status_events, 1, "1 status_changed after generating");

    // generating → completed
    let updated = ShippingDocService::transition_status(
        &pool,
        doc_req.id,
        tenant_id,
        &make_transition("completed"),
    )
    .await
    .expect("transition to completed");
    assert_eq!(updated.status, "completed");

    let status_events =
        count_outbox_events(&pool, &doc_req_id_str, "sr.shipping_doc.status_changed").await;
    assert_eq!(status_events, 2, "2 status_changed after completed");

    // completed is terminal — cannot transition further
    let result = ShippingDocService::transition_status(
        &pool,
        doc_req.id,
        tenant_id,
        &make_transition("generating"),
    )
    .await;
    assert!(
        result.is_err(),
        "completed is terminal — transition must be rejected"
    );

    // Now test the failure/retry path with a new doc request
    let req2 = make_doc_request(shipment_id, "packing_slip", None);
    let doc_req2 = ShippingDocService::create(&pool, tenant_id, &req2)
        .await
        .expect("create second doc request");
    let doc_req2_id_str = doc_req2.id.to_string();

    // requested → generating
    ShippingDocService::transition_status(
        &pool,
        doc_req2.id,
        tenant_id,
        &make_transition("generating"),
    )
    .await
    .expect("to generating");

    // generating → failed
    let updated = ShippingDocService::transition_status(
        &pool,
        doc_req2.id,
        tenant_id,
        &make_transition("failed"),
    )
    .await
    .expect("to failed");
    assert_eq!(updated.status, "failed");

    // failed → generating (retry)
    let updated = ShippingDocService::transition_status(
        &pool,
        doc_req2.id,
        tenant_id,
        &make_transition("generating"),
    )
    .await
    .expect("retry: failed → generating");
    assert_eq!(updated.status, "generating");

    let status_events2 =
        count_outbox_events(&pool, &doc_req2_id_str, "sr.shipping_doc.status_changed").await;
    assert_eq!(
        status_events2, 3,
        "3 status_changed events for the retry path (generating, failed, generating)"
    );

    // Verify event payloads have correct transition details
    let events: Vec<(serde_json::Value,)> = sqlx::query_as(
        r#"
        SELECT payload FROM sr_events_outbox
        WHERE aggregate_id = $1 AND event_type = 'sr.shipping_doc.status_changed'
        ORDER BY created_at
        "#,
    )
    .bind(&doc_req2_id_str)
    .fetch_all(&pool)
    .await
    .expect("fetch status events");

    assert_eq!(events.len(), 3);

    // First: requested → generating
    assert_eq!(events[0].0["from_status"], "requested");
    assert_eq!(events[0].0["to_status"], "generating");
    assert_eq!(events[0].0["tenant_id"], tenant_id.to_string());

    // Second: generating → failed
    assert_eq!(events[1].0["from_status"], "generating");
    assert_eq!(events[1].0["to_status"], "failed");

    // Third: failed → generating (retry)
    assert_eq!(events[2].0["from_status"], "failed");
    assert_eq!(events[2].0["to_status"], "generating");

    // Verify invalid transition: requested → completed (must go through generating)
    let req3 = make_doc_request(shipment_id, "packing_slip", None);
    let doc_req3 = ShippingDocService::create(&pool, tenant_id, &req3)
        .await
        .expect("create third doc request");

    let result = ShippingDocService::transition_status(
        &pool,
        doc_req3.id,
        tenant_id,
        &make_transition("completed"),
    )
    .await;
    assert!(
        result.is_err(),
        "requested → completed must be rejected (must go through generating)"
    );
}
