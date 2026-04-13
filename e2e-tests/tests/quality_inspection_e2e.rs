//! E2E tests: Quality Inspection module
//!
//! Covers:
//! 1. Inspection plan CRUD lifecycle (create, retrieve, activate)
//! 2. Inspection result recording — both pass and fail outcomes
//! 3. NCR (rejection) on failed inspection — hold → reject disposition
//! 4. Query inspections by part revision
//!
//! All tests hit real Postgres (quality-inspection DB port 5459,
//! workforce-competence DB port 5458). No mocks, no stubs.

use chrono::Utc;
use platform_sdk::PlatformClient;
use quality_inspection_rs::domain::models::*;
use quality_inspection_rs::domain::service;
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::time::Duration;
use uuid::Uuid;
use workforce_competence_rs::domain::{
    models::{ArtifactType, AssignCompetenceRequest, RegisterArtifactRequest},
    service as wc_service,
};

// ============================================================================
// DB setup
// ============================================================================

async fn qi_pool() -> PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("QUALITY_INSPECTION_DATABASE_URL").unwrap_or_else(|_| {
        "postgres://quality_inspection_user:quality_inspection_pass@localhost:5459/quality_inspection_db?sslmode=require".to_string()
    });
    let pool = PgPoolOptions::new()
        .max_connections(3)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&url)
        .await
        .expect("Failed to connect to quality-inspection DB");

    sqlx::migrate!("../modules/quality-inspection/db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run quality-inspection migrations");

    pool
}

async fn wc_pool() -> PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("WORKFORCE_COMPETENCE_DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://wc_user:wc_pass@localhost:5458/workforce_competence_db?sslmode=require"
            .to_string()
    });
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&url)
        .await
        .expect("Failed to connect to workforce-competence DB");

    sqlx::migrate!("../modules/workforce-competence/db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run workforce-competence migrations");

    pool
}

fn tenant() -> String {
    format!("qi-e2e-{}", Uuid::new_v4())
}

fn wc_http_client() -> PlatformClient {
    dotenvy::dotenv().ok();
    let base_url = std::env::var("WORKFORCE_COMPETENCE_BASE_URL")
        .unwrap_or_else(|_| "http://localhost:8121".to_string());
    PlatformClient::new(base_url)
}

async fn authorize_inspector(wc: &PgPool, tenant_id: &str, inspector_id: Uuid) {
    let artifact_req = RegisterArtifactRequest {
        tenant_id: tenant_id.to_string(),
        artifact_type: ArtifactType::Qualification,
        name: "Quality Inspection Disposition Authority".to_string(),
        code: "quality_inspection".to_string(),
        description: Some("E2E test inspector auth".to_string()),
        valid_duration_days: None,
        idempotency_key: Uuid::new_v4().to_string(),
        correlation_id: Some("e2e".to_string()),
        causation_id: None,
    };
    let (artifact, _) = wc_service::register_artifact(wc, &artifact_req)
        .await
        .expect("register quality_inspection artifact");

    let assign_req = AssignCompetenceRequest {
        tenant_id: tenant_id.to_string(),
        operator_id: inspector_id,
        artifact_id: artifact.id,
        awarded_at: Utc::now() - chrono::Duration::hours(1),
        expires_at: None,
        evidence_ref: Some("e2e-fixture".to_string()),
        awarded_by: Some("e2e-harness".to_string()),
        idempotency_key: Uuid::new_v4().to_string(),
        correlation_id: Some("e2e".to_string()),
        causation_id: None,
    };
    wc_service::assign_competence(wc, &assign_req)
        .await
        .expect("assign quality_inspection competence");
}

// ============================================================================
// Test 1: Inspection plan CRUD — create, get, activate
// ============================================================================

#[tokio::test]
#[serial]
async fn inspection_plan_create_get_activate() {
    let pool = qi_pool().await;
    let t = tenant();
    let part_id = Uuid::new_v4();
    let corr = Uuid::new_v4().to_string();

    // Create plan
    let req = CreateInspectionPlanRequest {
        part_id,
        plan_name: "Dimensional Check — Bracket A36".to_string(),
        revision: Some("B".to_string()),
        characteristics: vec![
            Characteristic {
                name: "Length".to_string(),
                characteristic_type: "dimensional".to_string(),
                key_characteristic: false,
                nominal: Some(150.0),
                tolerance_low: Some(149.5),
                tolerance_high: Some(150.5),
                uom: Some("mm".to_string()),
            },
            Characteristic {
                name: "Surface Roughness".to_string(),
                characteristic_type: "surface".to_string(),
                key_characteristic: false,
                nominal: Some(1.6),
                tolerance_low: None,
                tolerance_high: Some(3.2),
                uom: Some("Ra".to_string()),
            },
        ],
        sampling_method: Some("random".to_string()),
        sample_size: Some(5),
    };

    let plan = service::create_inspection_plan(&pool, &t, &req, &corr, None)
        .await
        .expect("create plan");

    assert_eq!(plan.tenant_id, t);
    assert_eq!(plan.part_id, part_id);
    assert_eq!(plan.plan_name, "Dimensional Check — Bracket A36");
    assert_eq!(plan.revision, "B");
    assert_eq!(plan.status, "draft");
    assert_eq!(plan.sampling_method, "random");
    assert_eq!(plan.sample_size, Some(5));

    let chars: Vec<Characteristic> =
        serde_json::from_value(plan.characteristics.clone()).expect("parse characteristics");
    assert_eq!(chars.len(), 2);
    assert_eq!(chars[0].name, "Length");

    // Get plan by ID
    let fetched = service::get_inspection_plan(&pool, &t, plan.id)
        .await
        .expect("get plan");
    assert_eq!(fetched.id, plan.id);
    assert_eq!(fetched.plan_name, plan.plan_name);

    // Activate plan
    let activated = service::activate_plan(&pool, &t, plan.id)
        .await
        .expect("activate plan");
    assert_eq!(activated.status, "active");

    // Activating again should fail (already active, not draft)
    let err = service::activate_plan(&pool, &t, plan.id).await;
    assert!(err.is_err(), "Re-activating an active plan must fail");
}

// ============================================================================
// Test 2: Inspection result recording — pass outcome
// ============================================================================

#[tokio::test]
#[serial]
async fn inspection_result_pass() {
    let pool = qi_pool().await;
    let t = tenant();
    let part_id = Uuid::new_v4();
    let receipt_id = Uuid::new_v4();
    let inspector_id = Uuid::new_v4();
    let corr = Uuid::new_v4().to_string();

    // Create a receiving inspection with result=pass
    let req = CreateReceivingInspectionRequest {
        plan_id: None,
        receipt_id: Some(receipt_id),
        lot_id: None,
        part_id: Some(part_id),
        part_revision: Some("A".to_string()),
        inspector_id: Some(inspector_id),
        result: Some("pass".to_string()),
        notes: Some("All dimensions within tolerance".to_string()),
    };

    let inspection = service::create_receiving_inspection(&pool, &t, &req, &corr, None)
        .await
        .expect("create receiving inspection");

    assert_eq!(inspection.inspection_type, "receiving");
    assert_eq!(inspection.result, "pass");
    assert_eq!(inspection.disposition, "pending");
    assert_eq!(inspection.receipt_id, Some(receipt_id));
    assert_eq!(inspection.part_id, Some(part_id));
    assert_eq!(inspection.part_revision.as_deref(), Some("A"));
    assert!(
        inspection.inspected_at.is_some(),
        "Pass result should set inspected_at"
    );

    // Verify retrievable by ID
    let fetched = service::get_inspection(&pool, &t, inspection.id)
        .await
        .expect("get inspection");
    assert_eq!(fetched.id, inspection.id);
    assert_eq!(fetched.result, "pass");

    // Verify retrievable by receipt
    let by_receipt = service::list_inspections_by_receipt(&pool, &t, receipt_id)
        .await
        .expect("list by receipt");
    assert_eq!(by_receipt.len(), 1);
    assert_eq!(by_receipt[0].id, inspection.id);
}

// ============================================================================
// Test 3: Inspection result recording — fail outcome + NCR via rejection
// ============================================================================

#[tokio::test]
#[serial]
async fn inspection_result_fail_and_ncr_rejection() {
    let pool = qi_pool().await;
    let wc = wc_pool().await;
    let wc_client = wc_http_client();
    let t = tenant();
    let part_id = Uuid::new_v4();
    let inspector_id = Uuid::new_v4();
    let corr = Uuid::new_v4().to_string();

    authorize_inspector(&wc, &t, inspector_id).await;

    // Create a receiving inspection with result=fail
    let req = CreateReceivingInspectionRequest {
        plan_id: None,
        receipt_id: Some(Uuid::new_v4()),
        lot_id: None,
        part_id: Some(part_id),
        part_revision: Some("C".to_string()),
        inspector_id: Some(inspector_id),
        result: Some("fail".to_string()),
        notes: Some("Surface corrosion detected — exceeds AS9100 limit".to_string()),
    };

    let inspection = service::create_receiving_inspection(&pool, &t, &req, &corr, None)
        .await
        .expect("create failed inspection");

    assert_eq!(inspection.result, "fail");
    assert_eq!(inspection.disposition, "pending");
    assert!(inspection.inspected_at.is_some());

    // Hold the failed inspection (quarantine)
    let held = service::hold_inspection(
        &pool,
        &wc_client,
        &t,
        inspection.id,
        Some(inspector_id),
        Some("Material quarantined pending NCR review"),
        &corr,
        None,
    )
    .await
    .expect("hold inspection");
    assert_eq!(held.disposition, "held");

    // Reject — this is the NCR action (non-conformance disposition)
    let rejected = service::reject_inspection(
        &pool,
        &wc_client,
        &t,
        inspection.id,
        Some(inspector_id),
        Some("NCR-2026-001: Rejected per AS9100 — surface corrosion"),
        &corr,
        None,
    )
    .await
    .expect("reject inspection");
    assert_eq!(rejected.disposition, "rejected");

    // Verify outbox has the full NCR event chain:
    // inspection_recorded → held → rejected
    let events: Vec<(String,)> = sqlx::query_as(
        "SELECT event_type FROM quality_inspection_outbox WHERE tenant_id = $1 ORDER BY created_at",
    )
    .bind(&t)
    .fetch_all(&pool)
    .await
    .unwrap();

    let types: Vec<&str> = events.iter().map(|r| r.0.as_str()).collect();
    assert_eq!(
        types,
        vec![
            "quality_inspection.inspection_recorded",
            "quality_inspection.held",
            "quality_inspection.rejected",
        ]
    );

    // Verify rejection event payload carries NCR reason
    let reject_payload: (serde_json::Value,) = sqlx::query_as(
        "SELECT payload FROM quality_inspection_outbox WHERE tenant_id = $1 AND event_type = 'quality_inspection.rejected'",
    )
    .bind(&t)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        reject_payload.0["payload"]["previous_disposition"], "held"
    );
    assert_eq!(reject_payload.0["payload"]["new_disposition"], "rejected");
    assert_eq!(
        reject_payload.0["payload"]["reason"],
        "NCR-2026-001: Rejected per AS9100 — surface corrosion"
    );
}

// ============================================================================
// Test 4: Full disposition state machine — pending → held → accepted
// ============================================================================

#[tokio::test]
#[serial]
async fn disposition_state_machine_accept() {
    let pool = qi_pool().await;
    let wc = wc_pool().await;
    let wc_client = wc_http_client();
    let t = tenant();
    let inspector_id = Uuid::new_v4();
    let corr = Uuid::new_v4().to_string();

    authorize_inspector(&wc, &t, inspector_id).await;

    // Create receiving inspection (pending)
    let req = CreateReceivingInspectionRequest {
        plan_id: None,
        receipt_id: Some(Uuid::new_v4()),
        lot_id: None,
        part_id: Some(Uuid::new_v4()),
        part_revision: None,
        inspector_id: None,
        result: None,
        notes: None,
    };

    let inspection = service::create_receiving_inspection(&pool, &t, &req, &corr, None)
        .await
        .expect("create inspection");
    assert_eq!(inspection.disposition, "pending");
    assert_eq!(inspection.result, "pending");

    // Invalid: pending → accepted should fail (must go through held)
    let err = service::accept_inspection(
        &pool,
        &wc_client,
        &t,
        inspection.id,
        Some(inspector_id),
        None,
        &corr,
        None,
    )
    .await;
    assert!(err.is_err(), "Cannot accept directly from pending");

    // Valid: pending → held
    let held = service::hold_inspection(
        &pool,
        &wc_client,
        &t,
        inspection.id,
        Some(inspector_id),
        Some("Awaiting measurement data"),
        &corr,
        None,
    )
    .await
    .expect("hold");
    assert_eq!(held.disposition, "held");

    // Valid: held → accepted
    let accepted = service::accept_inspection(
        &pool,
        &wc_client,
        &t,
        inspection.id,
        Some(inspector_id),
        Some("Measurements confirmed within spec"),
        &corr,
        None,
    )
    .await
    .expect("accept");
    assert_eq!(accepted.disposition, "accepted");

    // Terminal: accepted → anything should fail (no transitions from accepted)
    let err = service::hold_inspection(
        &pool,
        &wc_client,
        &t,
        inspection.id,
        Some(inspector_id),
        None,
        &corr,
        None,
    )
    .await;
    assert!(err.is_err(), "Cannot transition from accepted");
}

// ============================================================================
// Test 5: In-process and final inspection types
// ============================================================================

#[tokio::test]
#[serial]
async fn in_process_and_final_inspection_types() {
    let pool = qi_pool().await;
    let t = tenant();
    let wo_id = Uuid::new_v4();
    let op_id = Uuid::new_v4();
    let part_id = Uuid::new_v4();
    let corr = Uuid::new_v4().to_string();

    // In-process inspection
    let ip_req = CreateInProcessInspectionRequest {
        wo_id,
        op_instance_id: op_id,
        plan_id: None,
        lot_id: None,
        part_id: Some(part_id),
        part_revision: Some("D".to_string()),
        inspector_id: None,
        result: Some("pass".to_string()),
        notes: Some("CNC op passed dimensional check".to_string()),
    };

    let ip = service::create_in_process_inspection(&pool, &t, &ip_req, &corr, None)
        .await
        .expect("create in-process");
    assert_eq!(ip.inspection_type, "in_process");
    assert_eq!(ip.result, "pass");
    assert_eq!(ip.wo_id, Some(wo_id));
    assert_eq!(ip.op_instance_id, Some(op_id));

    // Final inspection
    let fi_req = CreateFinalInspectionRequest {
        wo_id,
        lot_id: None,
        plan_id: None,
        part_id: Some(part_id),
        part_revision: Some("D".to_string()),
        inspector_id: None,
        result: Some("fail".to_string()),
        notes: Some("Final surface finish out of spec".to_string()),
    };

    let fi = service::create_final_inspection(&pool, &t, &fi_req, &corr, None)
        .await
        .expect("create final");
    assert_eq!(fi.inspection_type, "final");
    assert_eq!(fi.result, "fail");
    assert_eq!(fi.wo_id, Some(wo_id));

    // Query by WO — should return both
    let by_wo = service::list_inspections_by_wo(&pool, &t, wo_id, None)
        .await
        .expect("list by wo");
    assert_eq!(by_wo.len(), 2);

    // Query by WO + type filter
    let by_wo_ip = service::list_inspections_by_wo(&pool, &t, wo_id, Some("in_process"))
        .await
        .expect("list by wo in_process");
    assert_eq!(by_wo_ip.len(), 1);
    assert_eq!(by_wo_ip[0].inspection_type, "in_process");

    let by_wo_fi = service::list_inspections_by_wo(&pool, &t, wo_id, Some("final"))
        .await
        .expect("list by wo final");
    assert_eq!(by_wo_fi.len(), 1);
    assert_eq!(by_wo_fi[0].inspection_type, "final");

    // Query by part — both have part_id
    let by_part = service::list_inspections_by_part_rev(&pool, &t, part_id, None)
        .await
        .expect("list by part");
    assert_eq!(by_part.len(), 2);

    // Query by part + revision filter
    let by_part_rev = service::list_inspections_by_part_rev(&pool, &t, part_id, Some("D"))
        .await
        .expect("list by part rev D");
    assert_eq!(by_part_rev.len(), 2);
}

// ============================================================================
// Test 6: Plan linked to inspection — plan_id flows through
// ============================================================================

#[tokio::test]
#[serial]
async fn plan_linked_to_inspection() {
    let pool = qi_pool().await;
    let t = tenant();
    let part_id = Uuid::new_v4();
    let corr = Uuid::new_v4().to_string();

    // Create and activate a plan
    let plan_req = CreateInspectionPlanRequest {
        part_id,
        plan_name: "Hardness Test Plan".to_string(),
        revision: None,
        characteristics: vec![Characteristic {
            name: "Rockwell Hardness".to_string(),
            characteristic_type: "hardness".to_string(),
            key_characteristic: false,
            nominal: Some(60.0),
            tolerance_low: Some(58.0),
            tolerance_high: Some(62.0),
            uom: Some("HRC".to_string()),
        }],
        sampling_method: None,
        sample_size: Some(3),
    };

    let plan = service::create_inspection_plan(&pool, &t, &plan_req, &corr, None)
        .await
        .expect("create plan");
    service::activate_plan(&pool, &t, plan.id)
        .await
        .expect("activate");

    // Create inspection referencing this plan
    let insp_req = CreateReceivingInspectionRequest {
        plan_id: Some(plan.id),
        receipt_id: Some(Uuid::new_v4()),
        lot_id: None,
        part_id: Some(part_id),
        part_revision: Some("A".to_string()),
        inspector_id: None,
        result: Some("pass".to_string()),
        notes: Some("Hardness 60.2 HRC — within spec".to_string()),
    };

    let inspection = service::create_receiving_inspection(&pool, &t, &insp_req, &corr, None)
        .await
        .expect("create inspection with plan");

    assert_eq!(inspection.plan_id, Some(plan.id));
    assert_eq!(inspection.part_id, Some(part_id));
    assert_eq!(inspection.result, "pass");
}
