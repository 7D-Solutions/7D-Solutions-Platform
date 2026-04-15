//! Phase C end-to-end proof: receiving → hold/release → in-process/final
//!
//! This test walks through the entire quality lifecycle in one flow:
//!   1. Inventory receipt event → auto-creates receiving inspection
//!   2. Hold the receiving inspection → release → verify disposition events
//!   3. Operation completed event → auto-creates in-process inspection
//!   4. FG receipt event → auto-creates final inspection
//!   5. Hold/accept the final inspection
//!   6. Evidence queries: by receipt, by WO, by part revision, by lot
//!
//! Each test uses a unique tenant_id to avoid cross-test interference.

use chrono::Utc;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use platform_sdk::PlatformClient;
use quality_inspection_rs::consumers::production_event_bridge::{
    process_fg_receipt_requested, process_operation_completed, FgReceiptRequestedPayload,
    OperationCompletedPayload,
};
use quality_inspection_rs::consumers::receipt_event_bridge::{
    process_item_received, ItemReceivedPayload,
};
use quality_inspection_rs::domain::service;
use serde::Serialize;
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

#[derive(Serialize)]
struct TestClaims {
    sub: String,
    iss: String,
    aud: String,
    iat: i64,
    exp: i64,
    jti: String,
    tenant_id: String,
    roles: Vec<String>,
    perms: Vec<String>,
    actor_type: String,
    ver: String,
}

fn sign_jwt(tenant_id: &str, perms: &[&str]) -> String {
    dotenvy::from_filename_override(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.env"),
    )
    .ok();
    let pem =
        std::env::var("JWT_PRIVATE_KEY_PEM").expect("JWT_PRIVATE_KEY_PEM must be set in .env");
    let encoding =
        EncodingKey::from_rsa_pem(pem.as_bytes()).expect("failed to parse JWT_PRIVATE_KEY_PEM");
    let now = Utc::now();
    let claims = TestClaims {
        sub: Uuid::new_v4().to_string(),
        iss: "auth-rs".to_string(),
        aud: "7d-platform".to_string(),
        iat: now.timestamp(),
        exp: (now + chrono::Duration::minutes(15)).timestamp(),
        jti: Uuid::new_v4().to_string(),
        tenant_id: tenant_id.to_string(),
        roles: vec!["operator".to_string()],
        perms: perms.iter().map(|s| s.to_string()).collect(),
        actor_type: "service".to_string(),
        ver: "1".to_string(),
    };
    jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, &encoding)
        .expect("failed to sign JWT")
}

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://quality_inspection_user:quality_inspection_pass@localhost:5459/quality_inspection_db?sslmode=require".to_string()
    });

    let pool = PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(&url)
        .await
        .expect("Failed to connect to quality-inspection test DB");

    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run quality-inspection migrations");

    pool
}

/// Register a "quality_inspection" artifact and assign competence to the given operator
/// via the running workforce-competence HTTP service.
async fn authorize_inspector(tenant_id: &str, inspector_id: Uuid) {
    let url = std::env::var("WORKFORCE_COMPETENCE_BASE_URL")
        .unwrap_or_else(|_| "http://localhost:8121".to_string());
    let token = sign_jwt(tenant_id, &["service.internal"]);
    let client = reqwest::Client::new();

    // Register the "quality_inspection" artifact
    let artifact_resp = client
        .post(format!("{url}/api/workforce-competence/artifacts"))
        .bearer_auth(&token)
        .header("x-tenant-id", tenant_id)
        .header("x-correlation-id", Uuid::new_v4().to_string())
        .header("x-actor-id", Uuid::nil().to_string())
        .json(&serde_json::json!({
            "tenant_id": tenant_id,
            "artifact_type": "qualification",
            "name": "Quality Inspection Disposition Authority",
            "code": "quality_inspection",
            "description": "Authorization to perform inspection dispositions",
            "valid_duration_days": null,
            "idempotency_key": Uuid::new_v4().to_string(),
            "correlation_id": "test",
            "causation_id": null
        }))
        .send()
        .await
        .expect("WC artifact registration HTTP call failed");

    let status = artifact_resp.status();
    assert!(
        status.is_success(),
        "register_artifact failed with {status}: {}",
        artifact_resp.text().await.unwrap_or_default()
    );

    let artifact: serde_json::Value = artifact_resp.json().await.expect("parse artifact response");
    let artifact_id = artifact["id"]
        .as_str()
        .expect("artifact.id must be a string");

    // Assign competence to the inspector
    let assign_resp = client
        .post(format!("{url}/api/workforce-competence/assignments"))
        .bearer_auth(&token)
        .header("x-tenant-id", tenant_id)
        .header("x-correlation-id", Uuid::new_v4().to_string())
        .header("x-actor-id", Uuid::nil().to_string())
        .json(&serde_json::json!({
            "tenant_id": tenant_id,
            "operator_id": inspector_id,
            "artifact_id": artifact_id,
            "awarded_at": (Utc::now() - chrono::Duration::hours(1)).to_rfc3339(),
            "expires_at": null,
            "evidence_ref": "test-fixture",
            "awarded_by": "test-harness",
            "idempotency_key": Uuid::new_v4().to_string(),
            "correlation_id": "test",
            "causation_id": null
        }))
        .send()
        .await
        .expect("WC assign competence HTTP call failed");

    let status = assign_resp.status();
    assert!(
        status.is_success(),
        "assign_competence failed with {status}: {}",
        assign_resp.text().await.unwrap_or_default()
    );
}

fn wc_client(tenant_id: &str) -> PlatformClient {
    let url = std::env::var("WORKFORCE_COMPETENCE_BASE_URL")
        .unwrap_or_else(|_| "http://localhost:8121".to_string());
    let token = sign_jwt(tenant_id, &["workforce_competence.read"]);
    PlatformClient::new(url).with_bearer_token(token)
}

fn unique_tenant() -> String {
    Uuid::new_v4().to_string()
}

// ============================================================================
// Full lifecycle: receipt → receiving inspection → hold → release → events
// then production ops → in-process inspection → FG receipt → final inspection
// → hold → accept → evidence queries
// ============================================================================

#[tokio::test]
#[serial]
async fn e2e_receiving_hold_release_in_process_final() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let wc = wc_client(&tenant);
    let corr = Uuid::new_v4().to_string();
    let part_id = Uuid::new_v4();
    let wo_id = Uuid::new_v4();
    let inspector_id = Uuid::new_v4();

    authorize_inspector(&tenant, inspector_id).await;

    // ── Step 1: Inventory receipt event → auto receiving inspection ──

    let receipt_event_id = Uuid::new_v4();
    let receipt_line_id = Uuid::new_v4();
    let receipt_payload = ItemReceivedPayload {
        receipt_line_id,
        tenant_id: tenant.clone(),
        item_id: part_id,
        sku: "BRACKET-A36-001".to_string(),
        warehouse_id: Uuid::new_v4(),
        quantity: 200,
        unit_cost_minor: 1500,
        currency: "USD".to_string(),
        source_type: "purchase".to_string(),
        purchase_order_id: Some(Uuid::new_v4()),
        received_at: Utc::now(),
    };

    let recv_insp_id = process_item_received(
        &pool,
        receipt_event_id,
        &tenant,
        &receipt_payload,
        &corr,
        None,
    )
    .await
    .expect("process_item_received should succeed")
    .expect("Should create a receiving inspection");

    // Verify receiving inspection exists with correct fields
    let recv_insp = service::get_inspection(&pool, &tenant, recv_insp_id)
        .await
        .expect("get receiving inspection");
    assert_eq!(recv_insp.inspection_type, "receiving");
    assert_eq!(recv_insp.result, "pending");
    assert_eq!(recv_insp.disposition, "pending");
    assert_eq!(recv_insp.receipt_id, Some(receipt_line_id));
    assert_eq!(recv_insp.part_id, Some(part_id));

    // ── Step 2: Hold the receiving inspection ──

    let held = service::hold_inspection(
        &pool,
        &wc,
        &tenant,
        recv_insp_id,
        Some(inspector_id),
        Some("Material quarantined pending dimensional check"),
        &corr,
        None,
    )
    .await
    .expect("hold_inspection should succeed");
    assert_eq!(held.disposition, "held");

    // ── Step 3: Release the receiving inspection ──

    let released = service::release_inspection(
        &pool,
        &wc,
        &tenant,
        recv_insp_id,
        Some(inspector_id),
        Some("Dimensions within tolerance, released for production"),
        &corr,
        None,
    )
    .await
    .expect("release_inspection should succeed");
    assert_eq!(released.disposition, "released");

    // ── Step 4: Verify disposition events in outbox ──
    // Expected so far: inspection_recorded + held + released = 3 events

    let outbox_events: Vec<(String,)> = sqlx::query_as(
        "SELECT event_type FROM quality_inspection_outbox WHERE tenant_id = $1 ORDER BY created_at",
    )
    .bind(&tenant)
    .fetch_all(&pool)
    .await
    .unwrap();

    let types: Vec<&str> = outbox_events.iter().map(|r| r.0.as_str()).collect();
    assert_eq!(
        types,
        vec![
            "quality_inspection.inspection_recorded",
            "quality_inspection.held",
            "quality_inspection.released",
        ],
        "Outbox should have inspection_recorded, held, released events"
    );

    // Verify released event payload has correct disposition transition
    let release_payload: (serde_json::Value,) = sqlx::query_as(
        "SELECT payload FROM quality_inspection_outbox WHERE tenant_id = $1 AND event_type = 'quality_inspection.released'",
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(release_payload.0["payload"]["previous_disposition"], "held");
    assert_eq!(release_payload.0["payload"]["new_disposition"], "released");
    assert_eq!(release_payload.0["source_module"], "quality-inspection");
    assert_eq!(release_payload.0["replay_safe"], true);

    // ── Step 5: Operation completed → auto in-process inspection ──

    let op_id_1 = Uuid::new_v4();
    let op_event_id_1 = Uuid::new_v4();
    let op_payload_1 = OperationCompletedPayload {
        operation_id: op_id_1,
        work_order_id: wo_id,
        tenant_id: tenant.clone(),
        operation_name: "CNC Machining".to_string(),
        sequence_number: 10,
    };

    let in_process_id_1 =
        process_operation_completed(&pool, op_event_id_1, &tenant, &op_payload_1, &corr, None)
            .await
            .expect("process_operation_completed should succeed")
            .expect("Should create in-process inspection for op 10");

    let in_proc_1 = service::get_inspection(&pool, &tenant, in_process_id_1)
        .await
        .expect("get in-process inspection");
    assert_eq!(in_proc_1.inspection_type, "in_process");
    assert_eq!(in_proc_1.result, "pending");
    assert_eq!(in_proc_1.wo_id, Some(wo_id));
    assert_eq!(in_proc_1.op_instance_id, Some(op_id_1));

    // Second operation on the same WO → separate in-process inspection
    let op_id_2 = Uuid::new_v4();
    let op_payload_2 = OperationCompletedPayload {
        operation_id: op_id_2,
        work_order_id: wo_id,
        tenant_id: tenant.clone(),
        operation_name: "Surface Grinding".to_string(),
        sequence_number: 20,
    };

    let in_process_id_2 =
        process_operation_completed(&pool, Uuid::new_v4(), &tenant, &op_payload_2, &corr, None)
            .await
            .expect("process op 2")
            .expect("Should create in-process inspection for op 20");

    assert_ne!(
        in_process_id_1, in_process_id_2,
        "Different ops should create different inspections"
    );

    // ── Step 6: FG receipt → auto final inspection ──

    let fg_event_id = Uuid::new_v4();
    let fg_payload = FgReceiptRequestedPayload {
        work_order_id: wo_id,
        tenant_id: tenant.clone(),
        order_number: "WO-2026-E2E".to_string(),
        item_id: part_id,
        warehouse_id: Uuid::new_v4(),
        quantity: 200,
        currency: "USD".to_string(),
    };

    let final_insp_id =
        process_fg_receipt_requested(&pool, fg_event_id, &tenant, &fg_payload, &corr, None)
            .await
            .expect("process_fg_receipt_requested should succeed")
            .expect("Should create final inspection");

    let final_insp = service::get_inspection(&pool, &tenant, final_insp_id)
        .await
        .expect("get final inspection");
    assert_eq!(final_insp.inspection_type, "final");
    assert_eq!(final_insp.result, "pending");
    assert_eq!(final_insp.wo_id, Some(wo_id));
    assert_eq!(final_insp.part_id, Some(part_id));

    // ── Step 7: Hold then accept the final inspection ──

    service::hold_inspection(
        &pool,
        &wc,
        &tenant,
        final_insp_id,
        Some(inspector_id),
        Some("Final hold for dimensional verification"),
        &corr,
        None,
    )
    .await
    .expect("hold final inspection");

    let accepted = service::accept_inspection(
        &pool,
        &wc,
        &tenant,
        final_insp_id,
        Some(inspector_id),
        Some("All checks passed — approved for shipment"),
        &corr,
        None,
    )
    .await
    .expect("accept final inspection");
    assert_eq!(accepted.disposition, "accepted");

    // ── Step 8: Evidence queries ──

    // 8a: Query by receipt — should return the receiving inspection
    let by_receipt = service::list_inspections_by_receipt(&pool, &tenant, receipt_line_id)
        .await
        .expect("list_by_receipt");
    assert_eq!(by_receipt.len(), 1);
    assert_eq!(by_receipt[0].id, recv_insp_id);
    assert_eq!(by_receipt[0].inspection_type, "receiving");

    // 8b: Query by WO — should return 2 in-process + 1 final = 3 inspections
    let by_wo_all = service::list_inspections_by_wo(&pool, &tenant, wo_id, None)
        .await
        .expect("list_by_wo all");
    assert_eq!(
        by_wo_all.len(),
        3,
        "WO should have 2 in-process + 1 final inspection"
    );

    let by_wo_in_process =
        service::list_inspections_by_wo(&pool, &tenant, wo_id, Some("in_process"))
            .await
            .expect("list_by_wo in_process");
    assert_eq!(by_wo_in_process.len(), 2);

    let by_wo_final = service::list_inspections_by_wo(&pool, &tenant, wo_id, Some("final"))
        .await
        .expect("list_by_wo final");
    assert_eq!(by_wo_final.len(), 1);
    assert_eq!(by_wo_final[0].id, final_insp_id);

    // 8c: Query by part — should return receiving + final = 2 (in-process has no part_id)
    let by_part = service::list_inspections_by_part_rev(&pool, &tenant, part_id, None)
        .await
        .expect("list_by_part");
    assert_eq!(
        by_part.len(),
        2,
        "Part should have receiving + final inspection"
    );
    let part_types: Vec<&str> = by_part.iter().map(|i| i.inspection_type.as_str()).collect();
    assert!(part_types.contains(&"receiving"));
    assert!(part_types.contains(&"final"));

    // ── Step 9: Verify full outbox event stream ──
    // Expected: receiving recorded + held + released
    //         + in-process recorded (op 10) + in-process recorded (op 20)
    //         + final recorded + final held + final accepted
    //         = 8 events total

    let all_events: Vec<(String,)> = sqlx::query_as(
        "SELECT event_type FROM quality_inspection_outbox WHERE tenant_id = $1 ORDER BY created_at",
    )
    .bind(&tenant)
    .fetch_all(&pool)
    .await
    .unwrap();

    assert_eq!(
        all_events.len(),
        8,
        "Should have 8 outbox events total for the full lifecycle"
    );

    let all_types: Vec<&str> = all_events.iter().map(|r| r.0.as_str()).collect();
    assert_eq!(
        all_types,
        vec![
            "quality_inspection.inspection_recorded", // receiving
            "quality_inspection.held",                // receiving held
            "quality_inspection.released",            // receiving released
            "quality_inspection.inspection_recorded", // in-process op 10
            "quality_inspection.inspection_recorded", // in-process op 20
            "quality_inspection.inspection_recorded", // final
            "quality_inspection.held",                // final held
            "quality_inspection.accepted",            // final accepted
        ]
    );

    // ── Step 10: Verify dedup table records ──

    let dedup_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM quality_inspection_processed_events WHERE event_id IN ($1, $2, $3)",
    )
    .bind(receipt_event_id)
    .bind(op_event_id_1)
    .bind(fg_event_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(
        dedup_count.0, 3,
        "All 3 bridge events should be recorded in dedup table"
    );
}

// ============================================================================
// Inspector authorization gate: unauthorized blocked, authorized succeeds
// ============================================================================

#[tokio::test]
#[serial]
async fn e2e_inspector_authorization_gate() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let wc = wc_client(&tenant);
    let corr = Uuid::new_v4().to_string();
    let unauthorized_inspector = Uuid::new_v4();
    let authorized_inspector = Uuid::new_v4();

    // ── Step 1: Receipt event → auto receiving inspection (no auth needed) ──

    let receipt_payload = ItemReceivedPayload {
        receipt_line_id: Uuid::new_v4(),
        tenant_id: tenant.clone(),
        item_id: Uuid::new_v4(),
        sku: "FLANGE-T7-002".to_string(),
        warehouse_id: Uuid::new_v4(),
        quantity: 50,
        unit_cost_minor: 3200,
        currency: "USD".to_string(),
        source_type: "purchase".to_string(),
        purchase_order_id: Some(Uuid::new_v4()),
        received_at: Utc::now(),
    };

    let recv_insp_id = process_item_received(
        &pool,
        Uuid::new_v4(),
        &tenant,
        &receipt_payload,
        &corr,
        None,
    )
    .await
    .expect("process_item_received should succeed")
    .expect("Should create a receiving inspection");

    // ── Step 2: Attempt hold with UNAUTHORIZED inspector → must fail ──

    let hold_err = service::hold_inspection(
        &pool,
        &wc,
        &tenant,
        recv_insp_id,
        Some(unauthorized_inspector),
        Some("Attempting hold without authorization"),
        &corr,
        None,
    )
    .await;

    assert!(hold_err.is_err(), "Unauthorized inspector must be rejected");
    let err_msg = hold_err.unwrap_err().to_string();
    assert!(
        err_msg.contains("not authorized"),
        "Error should indicate authorization failure, got: {}",
        err_msg
    );

    // Verify inspection is still pending (unauthorized hold had no effect)
    let still_pending = service::get_inspection(&pool, &tenant, recv_insp_id)
        .await
        .expect("get inspection");
    assert_eq!(
        still_pending.disposition, "pending",
        "Disposition must remain pending after unauthorized attempt"
    );

    // ── Step 3: Grant authority to the authorized inspector via WC HTTP API ──

    authorize_inspector(&tenant, authorized_inspector).await;

    // ── Step 4: Hold with AUTHORIZED inspector → must succeed ──

    let held = service::hold_inspection(
        &pool,
        &wc,
        &tenant,
        recv_insp_id,
        Some(authorized_inspector),
        Some("Material quarantined — authorized inspector"),
        &corr,
        None,
    )
    .await
    .expect("Authorized inspector should be able to hold");
    assert_eq!(held.disposition, "held");

    // ── Step 5: Release with AUTHORIZED inspector → must succeed ──

    let released = service::release_inspection(
        &pool,
        &wc,
        &tenant,
        recv_insp_id,
        Some(authorized_inspector),
        Some("Dimensional check passed — released"),
        &corr,
        None,
    )
    .await
    .expect("Authorized inspector should be able to release");
    assert_eq!(released.disposition, "released");

    // ── Step 6: Verify outbox events include inspector_id ──

    let held_event: (serde_json::Value,) = sqlx::query_as(
        "SELECT payload FROM quality_inspection_outbox WHERE tenant_id = $1 AND event_type = 'quality_inspection.held'",
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        held_event.0["payload"]["inspector_id"],
        authorized_inspector.to_string(),
        "Held event must record authorized inspector_id"
    );

    let released_event: (serde_json::Value,) = sqlx::query_as(
        "SELECT payload FROM quality_inspection_outbox WHERE tenant_id = $1 AND event_type = 'quality_inspection.released'",
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        released_event.0["payload"]["inspector_id"],
        authorized_inspector.to_string(),
        "Released event must record authorized inspector_id"
    );

    // ── Step 7: Verify NO outbox events from the unauthorized attempt ──
    // Expected: inspection_recorded (from receipt) + held + released = 3 events only

    let all_events: Vec<(String,)> = sqlx::query_as(
        "SELECT event_type FROM quality_inspection_outbox WHERE tenant_id = $1 ORDER BY created_at",
    )
    .bind(&tenant)
    .fetch_all(&pool)
    .await
    .unwrap();

    assert_eq!(
        all_events.len(),
        3,
        "Only 3 events expected — unauthorized attempt must not produce events"
    );
}

// ============================================================================
// Quarantine round-trip: receive → hold → reject (material blocked from use)
// ============================================================================

#[tokio::test]
#[serial]
async fn e2e_quarantine_round_trip_reject() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let wc = wc_client(&tenant);
    let corr = Uuid::new_v4().to_string();
    let inspector_id = Uuid::new_v4();

    authorize_inspector(&tenant, inspector_id).await;

    // Receipt event → auto receiving inspection
    let receipt_payload = ItemReceivedPayload {
        receipt_line_id: Uuid::new_v4(),
        tenant_id: tenant.clone(),
        item_id: Uuid::new_v4(),
        sku: "RIVET-SS-004".to_string(),
        warehouse_id: Uuid::new_v4(),
        quantity: 500,
        unit_cost_minor: 50,
        currency: "USD".to_string(),
        source_type: "purchase".to_string(),
        purchase_order_id: Some(Uuid::new_v4()),
        received_at: Utc::now(),
    };

    let recv_id = process_item_received(
        &pool,
        Uuid::new_v4(),
        &tenant,
        &receipt_payload,
        &corr,
        None,
    )
    .await
    .expect("process receipt")
    .expect("should create inspection");

    // Hold — material quarantined
    service::hold_inspection(
        &pool,
        &wc,
        &tenant,
        recv_id,
        Some(inspector_id),
        Some("Visual defects found — corrosion on surface"),
        &corr,
        None,
    )
    .await
    .expect("hold");

    // Reject — material fails inspection
    let rejected = service::reject_inspection(
        &pool,
        &wc,
        &tenant,
        recv_id,
        Some(inspector_id),
        Some("Rejected per AS9100 — surface corrosion exceeds limit"),
        &corr,
        None,
    )
    .await
    .expect("reject");

    assert_eq!(rejected.disposition, "rejected");

    // Verify outbox: recorded + held + rejected
    let event_types: Vec<(String,)> = sqlx::query_as(
        "SELECT event_type FROM quality_inspection_outbox WHERE tenant_id = $1 ORDER BY created_at",
    )
    .bind(&tenant)
    .fetch_all(&pool)
    .await
    .unwrap();

    let types: Vec<&str> = event_types.iter().map(|r| r.0.as_str()).collect();
    assert_eq!(
        types,
        vec![
            "quality_inspection.inspection_recorded",
            "quality_inspection.held",
            "quality_inspection.rejected",
        ]
    );

    // Verify rejection event payload
    let reject_payload: (serde_json::Value,) = sqlx::query_as(
        "SELECT payload FROM quality_inspection_outbox WHERE tenant_id = $1 AND event_type = 'quality_inspection.rejected'",
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(reject_payload.0["payload"]["previous_disposition"], "held");
    assert_eq!(reject_payload.0["payload"]["new_disposition"], "rejected");
    assert_eq!(
        reject_payload.0["payload"]["reason"],
        "Rejected per AS9100 — surface corrosion exceeds limit"
    );
}

// ============================================================================
// Idempotency: duplicate events do not create duplicate inspections
// ============================================================================

#[tokio::test]
#[serial]
async fn e2e_idempotency_across_all_bridges() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();
    let wo_id = Uuid::new_v4();

    // Receipt bridge: same event_id twice → only 1 inspection
    let receipt_event_id = Uuid::new_v4();
    let receipt_payload = ItemReceivedPayload {
        receipt_line_id: Uuid::new_v4(),
        tenant_id: tenant.clone(),
        item_id: Uuid::new_v4(),
        sku: "WASHER-FL-010".to_string(),
        warehouse_id: Uuid::new_v4(),
        quantity: 1000,
        unit_cost_minor: 20,
        currency: "USD".to_string(),
        source_type: "purchase".to_string(),
        purchase_order_id: Some(Uuid::new_v4()),
        received_at: Utc::now(),
    };

    let first = process_item_received(
        &pool,
        receipt_event_id,
        &tenant,
        &receipt_payload,
        &corr,
        None,
    )
    .await
    .unwrap();
    assert!(first.is_some());

    let dup = process_item_received(
        &pool,
        receipt_event_id,
        &tenant,
        &receipt_payload,
        &corr,
        None,
    )
    .await
    .unwrap();
    assert!(dup.is_none(), "Duplicate receipt event should be skipped");

    // Op completed bridge: same event_id twice → only 1 inspection
    let op_event_id = Uuid::new_v4();
    let op_payload = OperationCompletedPayload {
        operation_id: Uuid::new_v4(),
        work_order_id: wo_id,
        tenant_id: tenant.clone(),
        operation_name: "Deburring".to_string(),
        sequence_number: 30,
    };

    let first_op =
        process_operation_completed(&pool, op_event_id, &tenant, &op_payload, &corr, None)
            .await
            .unwrap();
    assert!(first_op.is_some());

    let dup_op = process_operation_completed(&pool, op_event_id, &tenant, &op_payload, &corr, None)
        .await
        .unwrap();
    assert!(dup_op.is_none(), "Duplicate op event should be skipped");

    // FG receipt bridge: same event_id twice → only 1 inspection
    let fg_event_id = Uuid::new_v4();
    let fg_payload = FgReceiptRequestedPayload {
        work_order_id: wo_id,
        tenant_id: tenant.clone(),
        order_number: "WO-DEDUP".to_string(),
        item_id: Uuid::new_v4(),
        warehouse_id: Uuid::new_v4(),
        quantity: 100,
        currency: "USD".to_string(),
    };

    let first_fg =
        process_fg_receipt_requested(&pool, fg_event_id, &tenant, &fg_payload, &corr, None)
            .await
            .unwrap();
    assert!(first_fg.is_some());

    let dup_fg =
        process_fg_receipt_requested(&pool, fg_event_id, &tenant, &fg_payload, &corr, None)
            .await
            .unwrap();
    assert!(dup_fg.is_none(), "Duplicate FG event should be skipped");

    // Verify exactly 3 inspections for this tenant
    let total: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM inspections WHERE tenant_id = $1")
        .bind(&tenant)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        total.0, 3,
        "Should have exactly 3 inspections despite 6 event processing attempts"
    );
}
