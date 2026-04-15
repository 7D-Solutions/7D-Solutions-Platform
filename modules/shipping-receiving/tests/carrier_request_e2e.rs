//! E2E integration tests: Carrier integration framework (bd-1qsu8).
//!
//! Tests run against a real PostgreSQL database on port 5454. Proves:
//! 1. Rate request E2E — create, transition to completed with response, verify persistence
//! 2. Label request E2E — create, verify lifecycle and status tracking
//! 3. Track request E2E — create, verify status updates
//! 4. Tenant isolation — tenant_A data invisible to tenant_B
//! 5. Idempotency — same idempotency_key produces no duplicate
//! 6. Outbox event — each lifecycle transition emits outbox event

use serial_test::serial;
use shipping_receiving_rs::domain::carrier_requests::{
    CarrierRequestService, CreateCarrierRequest, TransitionCarrierRequest,
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

fn make_carrier_request(
    shipment_id: Uuid,
    request_type: &str,
    carrier_code: &str,
    idem_key: Option<&str>,
) -> CreateCarrierRequest {
    CreateCarrierRequest {
        shipment_id,
        request_type: request_type.to_string(),
        carrier_code: carrier_code.to_string(),
        payload: serde_json::json!({
            "origin_zip": "90210",
            "dest_zip": "10001",
            "weight_lbs": 25,
        }),
        idempotency_key: idem_key.map(|s| s.to_string()),
    }
}

fn make_transition(status: &str, response: Option<serde_json::Value>) -> TransitionCarrierRequest {
    TransitionCarrierRequest {
        status: status.to_string(),
        response,
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

// ── 1. Rate request E2E ─────────────────────────────────────

#[tokio::test]
#[serial]
async fn rate_request_create_transition_to_completed_with_response() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let shipment_id = Uuid::new_v4();

    // Create rate request
    let req = make_carrier_request(shipment_id, "rate", "fedex", None);
    let cr = CarrierRequestService::create(&pool, tenant_id, &req)
        .await
        .expect("create rate request");

    assert_eq!(cr.tenant_id, tenant_id);
    assert_eq!(cr.shipment_id, shipment_id);
    assert_eq!(cr.request_type, "rate");
    assert_eq!(cr.carrier_code, "fedex");
    assert_eq!(cr.status, "pending");
    assert!(cr.response.is_none());

    // Verify findable
    let found = CarrierRequestService::find_by_id(&pool, cr.id, tenant_id)
        .await
        .expect("find by id");
    assert!(found.is_some());
    assert_eq!(found.unwrap().request_type, "rate");

    // pending → submitted
    let updated = CarrierRequestService::transition_status(
        &pool,
        cr.id,
        tenant_id,
        &make_transition("submitted", None),
    )
    .await
    .expect("transition to submitted");
    assert_eq!(updated.status, "submitted");

    // submitted → completed with response data
    let rate_response = serde_json::json!({
        "rates": [
            {"service": "ground", "cost_cents": 1250, "transit_days": 5},
            {"service": "express", "cost_cents": 3500, "transit_days": 2},
        ]
    });
    let updated = CarrierRequestService::transition_status(
        &pool,
        cr.id,
        tenant_id,
        &make_transition("completed", Some(rate_response.clone())),
    )
    .await
    .expect("transition to completed");
    assert_eq!(updated.status, "completed");
    assert_eq!(updated.response, Some(rate_response));

    // completed is terminal — cannot transition further
    let result = CarrierRequestService::transition_status(
        &pool,
        cr.id,
        tenant_id,
        &make_transition("submitted", None),
    )
    .await;
    assert!(
        result.is_err(),
        "completed is terminal — transition must be rejected"
    );
}

// ── 2. Label request E2E ────────────────────────────────────

#[tokio::test]
#[serial]
async fn label_request_lifecycle_and_status_tracking() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let shipment_id = Uuid::new_v4();

    // Create label request
    let req = make_carrier_request(shipment_id, "label", "ups", None);
    let cr = CarrierRequestService::create(&pool, tenant_id, &req)
        .await
        .expect("create label request");

    assert_eq!(cr.request_type, "label");
    assert_eq!(cr.carrier_code, "ups");
    assert_eq!(cr.status, "pending");

    // pending → submitted
    let updated = CarrierRequestService::transition_status(
        &pool,
        cr.id,
        tenant_id,
        &make_transition("submitted", None),
    )
    .await
    .expect("to submitted");
    assert_eq!(updated.status, "submitted");

    // submitted → failed (carrier rejected)
    let updated = CarrierRequestService::transition_status(
        &pool,
        cr.id,
        tenant_id,
        &make_transition(
            "failed",
            Some(serde_json::json!({"error": "invalid address"})),
        ),
    )
    .await
    .expect("to failed");
    assert_eq!(updated.status, "failed");

    // failed → submitted (retry)
    let updated = CarrierRequestService::transition_status(
        &pool,
        cr.id,
        tenant_id,
        &make_transition("submitted", None),
    )
    .await
    .expect("retry: failed → submitted");
    assert_eq!(updated.status, "submitted");

    // submitted → completed with label data
    let label_response = serde_json::json!({
        "tracking_number": "1Z999AA10123456784",
        "label_url": "https://labels.ups.com/1Z999AA10123456784.pdf",
    });
    let updated = CarrierRequestService::transition_status(
        &pool,
        cr.id,
        tenant_id,
        &make_transition("completed", Some(label_response.clone())),
    )
    .await
    .expect("to completed");
    assert_eq!(updated.status, "completed");
    assert_eq!(updated.response, Some(label_response));

    // Verify list_by_shipment returns this request
    let list = CarrierRequestService::list_by_shipment(&pool, shipment_id, tenant_id)
        .await
        .expect("list by shipment");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, cr.id);
}

// ── 3. Track request E2E ────────────────────────────────────

#[tokio::test]
#[serial]
async fn track_request_status_updates() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let shipment_id = Uuid::new_v4();

    // Create track request
    let req = make_carrier_request(shipment_id, "track", "usps", None);
    let cr = CarrierRequestService::create(&pool, tenant_id, &req)
        .await
        .expect("create track request");

    assert_eq!(cr.request_type, "track");
    assert_eq!(cr.carrier_code, "usps");
    assert_eq!(cr.status, "pending");

    // pending → submitted
    let updated = CarrierRequestService::transition_status(
        &pool,
        cr.id,
        tenant_id,
        &make_transition("submitted", None),
    )
    .await
    .expect("to submitted");
    assert_eq!(updated.status, "submitted");

    // submitted → completed with tracking data
    let track_response = serde_json::json!({
        "tracking_number": "9400111899223456789012",
        "events": [
            {"timestamp": "2026-03-01T10:00:00Z", "location": "Los Angeles, CA", "status": "In Transit"},
            {"timestamp": "2026-03-03T14:30:00Z", "location": "New York, NY", "status": "Delivered"},
        ]
    });
    let updated = CarrierRequestService::transition_status(
        &pool,
        cr.id,
        tenant_id,
        &make_transition("completed", Some(track_response.clone())),
    )
    .await
    .expect("to completed");
    assert_eq!(updated.status, "completed");
    assert_eq!(updated.response, Some(track_response));

    // Invalid transition: pending → completed (skipping submitted)
    let req2 = make_carrier_request(shipment_id, "track", "usps", None);
    let cr2 = CarrierRequestService::create(&pool, tenant_id, &req2)
        .await
        .expect("create second track request");

    let result = CarrierRequestService::transition_status(
        &pool,
        cr2.id,
        tenant_id,
        &make_transition("completed", None),
    )
    .await;
    assert!(
        result.is_err(),
        "pending → completed must be rejected (must go through submitted)"
    );
}

// ── 4. Tenant isolation ─────────────────────────────────────

#[tokio::test]
#[serial]
async fn tenant_isolation_carrier_requests_invisible_across_tenants() {
    let pool = setup_db().await;
    let tenant_a = Uuid::new_v4();
    let tenant_b = Uuid::new_v4();
    let shipment_id = Uuid::new_v4();

    // Create request under tenant_A
    let req = make_carrier_request(shipment_id, "rate", "fedex", None);
    let cr = CarrierRequestService::create(&pool, tenant_a, &req)
        .await
        .expect("create under tenant_A");

    // Query as tenant_B — must return None
    let found = CarrierRequestService::find_by_id(&pool, cr.id, tenant_b)
        .await
        .expect("find_by_id as tenant_B");
    assert!(
        found.is_none(),
        "tenant_B must not see tenant_A's carrier request"
    );

    // List by shipment as tenant_B — must return empty
    let list = CarrierRequestService::list_by_shipment(&pool, shipment_id, tenant_b)
        .await
        .expect("list as tenant_B");
    assert!(
        list.is_empty(),
        "tenant_B must not see tenant_A's carrier requests"
    );

    // Status transition as tenant_B must fail (not found)
    let result = CarrierRequestService::transition_status(
        &pool,
        cr.id,
        tenant_b,
        &make_transition("submitted", None),
    )
    .await;
    assert!(
        result.is_err(),
        "tenant_B must not be able to transition tenant_A's carrier request"
    );
}

// ── 5. Idempotency ──────────────────────────────────────────

#[tokio::test]
#[serial]
async fn idempotent_carrier_request_no_duplicate() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let shipment_id = Uuid::new_v4();
    let idem_key = "carrier-idem-001";

    // First create
    let req = make_carrier_request(shipment_id, "label", "fedex", Some(idem_key));
    let cr1 = CarrierRequestService::create(&pool, tenant_id, &req)
        .await
        .expect("first create");

    // Second create with same idempotency key
    let req2 = make_carrier_request(shipment_id, "label", "fedex", Some(idem_key));
    let cr2 = CarrierRequestService::create(&pool, tenant_id, &req2)
        .await
        .expect("second create (idempotent)");

    // Must return the same request, not a new one
    assert_eq!(
        cr1.id, cr2.id,
        "idempotent create must return same carrier request"
    );

    // Verify only one request exists for this key/tenant
    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM sr_carrier_requests WHERE tenant_id = $1 AND idempotency_key = $2",
    )
    .bind(tenant_id)
    .bind(idem_key)
    .fetch_one(&pool)
    .await
    .expect("count requests");
    assert_eq!(count.0, 1, "must have exactly one request, not a duplicate");
}

// ── 6. Outbox events ────────────────────────────────────────

#[tokio::test]
#[serial]
async fn outbox_events_emitted_on_each_lifecycle_transition() {
    let pool = setup_db().await;
    let tenant_id = Uuid::new_v4();
    let shipment_id = Uuid::new_v4();

    // Create carrier request
    let req = make_carrier_request(shipment_id, "rate", "dhl", None);
    let cr = CarrierRequestService::create(&pool, tenant_id, &req)
        .await
        .expect("create carrier request");
    let cr_id_str = cr.id.to_string();

    // Verify creation outbox event
    let created_events = count_outbox_events(&pool, &cr_id_str, "sr.carrier_request.created").await;
    assert_eq!(
        created_events, 1,
        "must have exactly 1 carrier_request.created outbox event"
    );

    // Verify creation event has correct payload
    let events: Vec<(serde_json::Value, String)> = sqlx::query_as(
        r#"
        SELECT payload, tenant_id FROM sr_events_outbox
        WHERE aggregate_id = $1 AND event_type = 'sr.carrier_request.created'
        "#,
    )
    .bind(&cr_id_str)
    .fetch_all(&pool)
    .await
    .expect("fetch creation events");

    assert_eq!(events.len(), 1);
    let (payload, outbox_tenant_id) = &events[0];
    assert_eq!(outbox_tenant_id, &tenant_id.to_string());
    assert_eq!(payload["request_type"], "rate");
    assert_eq!(payload["carrier_code"], "dhl");
    assert_eq!(payload["tenant_id"], tenant_id.to_string());

    // pending → submitted
    CarrierRequestService::transition_status(
        &pool,
        cr.id,
        tenant_id,
        &make_transition("submitted", None),
    )
    .await
    .expect("to submitted");

    let status_events =
        count_outbox_events(&pool, &cr_id_str, "sr.carrier_request.status_changed").await;
    assert_eq!(status_events, 1, "1 status_changed after submitted");

    // submitted → completed
    CarrierRequestService::transition_status(
        &pool,
        cr.id,
        tenant_id,
        &make_transition("completed", Some(serde_json::json!({"rate": 12.50}))),
    )
    .await
    .expect("to completed");

    let status_events =
        count_outbox_events(&pool, &cr_id_str, "sr.carrier_request.status_changed").await;
    assert_eq!(status_events, 2, "2 status_changed after completed");

    // Verify event payloads have correct transition details
    let transition_events: Vec<(serde_json::Value,)> = sqlx::query_as(
        r#"
        SELECT payload FROM sr_events_outbox
        WHERE aggregate_id = $1 AND event_type = 'sr.carrier_request.status_changed'
        ORDER BY created_at
        "#,
    )
    .bind(&cr_id_str)
    .fetch_all(&pool)
    .await
    .expect("fetch status events");

    assert_eq!(transition_events.len(), 2);

    // First: pending → submitted
    assert_eq!(transition_events[0].0["from_status"], "pending");
    assert_eq!(transition_events[0].0["to_status"], "submitted");
    assert_eq!(transition_events[0].0["tenant_id"], tenant_id.to_string());

    // Second: submitted → completed
    assert_eq!(transition_events[1].0["from_status"], "submitted");
    assert_eq!(transition_events[1].0["to_status"], "completed");

    // Now test failure/retry path with a new request
    let req2 = make_carrier_request(shipment_id, "label", "dhl", None);
    let cr2 = CarrierRequestService::create(&pool, tenant_id, &req2)
        .await
        .expect("create second request");
    let cr2_id_str = cr2.id.to_string();

    // pending → submitted → failed → submitted
    CarrierRequestService::transition_status(
        &pool,
        cr2.id,
        tenant_id,
        &make_transition("submitted", None),
    )
    .await
    .expect("to submitted");

    CarrierRequestService::transition_status(
        &pool,
        cr2.id,
        tenant_id,
        &make_transition("failed", Some(serde_json::json!({"error": "timeout"}))),
    )
    .await
    .expect("to failed");

    CarrierRequestService::transition_status(
        &pool,
        cr2.id,
        tenant_id,
        &make_transition("submitted", None),
    )
    .await
    .expect("retry: failed → submitted");

    let retry_events =
        count_outbox_events(&pool, &cr2_id_str, "sr.carrier_request.status_changed").await;
    assert_eq!(
        retry_events, 3,
        "3 status_changed events for the retry path (submitted, failed, submitted)"
    );
}
