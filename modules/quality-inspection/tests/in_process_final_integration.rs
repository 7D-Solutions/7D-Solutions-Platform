use chrono::Utc;
use quality_inspection_rs::domain::models::*;
use quality_inspection_rs::domain::service;
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;
use workforce_competence_rs::domain::{
    models::{ArtifactType, AssignCompetenceRequest, RegisterArtifactRequest},
    service as wc_service,
};

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

async fn setup_wc_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("WORKFORCE_COMPETENCE_DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://wc_user:wc_pass@localhost:5458/workforce_competence_db?sslmode=require"
            .to_string()
    });

    let pool = PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(&url)
        .await
        .expect("Failed to connect to workforce-competence test DB");

    sqlx::migrate!("../workforce-competence/db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run workforce-competence migrations");

    pool
}

async fn authorize_inspector(wc_pool: &sqlx::PgPool, tenant_id: &str, inspector_id: Uuid) {
    let artifact_req = RegisterArtifactRequest {
        tenant_id: tenant_id.to_string(),
        artifact_type: ArtifactType::Qualification,
        name: "Quality Inspection Disposition Authority".to_string(),
        code: "quality_inspection".to_string(),
        description: Some("Authorization to perform inspection dispositions".to_string()),
        valid_duration_days: None,
        idempotency_key: Uuid::new_v4().to_string(),
        correlation_id: Some("test".to_string()),
        causation_id: None,
    };

    let (artifact, _) = wc_service::register_artifact(wc_pool, &artifact_req)
        .await
        .expect("register quality_inspection artifact");

    let assign_req = AssignCompetenceRequest {
        tenant_id: tenant_id.to_string(),
        operator_id: inspector_id,
        artifact_id: artifact.id,
        awarded_at: Utc::now() - chrono::Duration::hours(1),
        expires_at: None,
        evidence_ref: Some("test-fixture".to_string()),
        awarded_by: Some("test-harness".to_string()),
        idempotency_key: Uuid::new_v4().to_string(),
        correlation_id: Some("test".to_string()),
        causation_id: None,
    };

    wc_service::assign_competence(wc_pool, &assign_req)
        .await
        .expect("assign quality_inspection competence");
}

fn unique_tenant() -> String {
    format!("test-tenant-{}", Uuid::new_v4())
}

// ============================================================================
// Create in-process inspection linked to WO + op instance
// ============================================================================

#[tokio::test]
#[serial]
async fn create_in_process_inspection_linked_to_wo_and_op() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();
    let wo_id = Uuid::new_v4();
    let op_instance_id = Uuid::new_v4();
    let part_id = Uuid::new_v4();

    let inspection = service::create_in_process_inspection(
        &pool,
        &tenant,
        &CreateInProcessInspectionRequest {
            wo_id,
            op_instance_id,
            plan_id: None,
            lot_id: None,
            part_id: Some(part_id),
            part_revision: Some("A".to_string()),
            inspector_id: None,
            result: Some("pass".to_string()),
            notes: Some("In-process check after machining".to_string()),
        },
        &corr,
        None,
    )
    .await
    .expect("create_in_process_inspection");

    assert_eq!(inspection.tenant_id, tenant);
    assert_eq!(inspection.inspection_type, "in_process");
    assert_eq!(inspection.result, "pass");
    assert_eq!(inspection.wo_id, Some(wo_id));
    assert_eq!(inspection.op_instance_id, Some(op_instance_id));
    assert_eq!(inspection.part_id, Some(part_id));
    assert_eq!(inspection.part_revision.as_deref(), Some("A"));
    assert!(inspection.inspected_at.is_some());
}

// ============================================================================
// Create final inspection linked to WO + produced lot
// ============================================================================

#[tokio::test]
#[serial]
async fn create_final_inspection_linked_to_wo_and_lot() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();
    let wo_id = Uuid::new_v4();
    let produced_lot_id = Uuid::new_v4();
    let part_id = Uuid::new_v4();

    let inspection = service::create_final_inspection(
        &pool,
        &tenant,
        &CreateFinalInspectionRequest {
            wo_id,
            lot_id: Some(produced_lot_id),
            plan_id: None,
            part_id: Some(part_id),
            part_revision: Some("B".to_string()),
            inspector_id: None,
            result: Some("pass".to_string()),
            notes: Some("Final inspection before shipment".to_string()),
        },
        &corr,
        None,
    )
    .await
    .expect("create_final_inspection");

    assert_eq!(inspection.tenant_id, tenant);
    assert_eq!(inspection.inspection_type, "final");
    assert_eq!(inspection.result, "pass");
    assert_eq!(inspection.wo_id, Some(wo_id));
    assert_eq!(inspection.lot_id, Some(produced_lot_id));
    assert_eq!(inspection.part_id, Some(part_id));
    assert_eq!(inspection.part_revision.as_deref(), Some("B"));
    assert!(inspection.inspected_at.is_some());
}

// ============================================================================
// Query by WO — returns in-process + final
// ============================================================================

#[tokio::test]
#[serial]
async fn query_inspections_by_wo() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();
    let wo_id = Uuid::new_v4();
    let part_id = Uuid::new_v4();

    // Create in-process inspection
    service::create_in_process_inspection(
        &pool,
        &tenant,
        &CreateInProcessInspectionRequest {
            wo_id,
            op_instance_id: Uuid::new_v4(),
            plan_id: None,
            lot_id: None,
            part_id: Some(part_id),
            part_revision: Some("A".to_string()),
            inspector_id: None,
            result: Some("pass".to_string()),
            notes: None,
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    // Create final inspection for same WO
    service::create_final_inspection(
        &pool,
        &tenant,
        &CreateFinalInspectionRequest {
            wo_id,
            lot_id: Some(Uuid::new_v4()),
            plan_id: None,
            part_id: Some(part_id),
            part_revision: Some("A".to_string()),
            inspector_id: None,
            result: Some("pass".to_string()),
            notes: None,
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    // Query all inspections for this WO
    let all = service::list_inspections_by_wo(&pool, &tenant, wo_id, None)
        .await
        .unwrap();
    assert_eq!(all.len(), 2);

    // Query only in_process
    let in_process =
        service::list_inspections_by_wo(&pool, &tenant, wo_id, Some("in_process"))
            .await
            .unwrap();
    assert_eq!(in_process.len(), 1);
    assert_eq!(in_process[0].inspection_type, "in_process");

    // Query only final
    let finals = service::list_inspections_by_wo(&pool, &tenant, wo_id, Some("final"))
        .await
        .unwrap();
    assert_eq!(finals.len(), 1);
    assert_eq!(finals[0].inspection_type, "final");
}

// ============================================================================
// Query by lot
// ============================================================================

#[tokio::test]
#[serial]
async fn query_inspections_by_lot() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();
    let lot_id = Uuid::new_v4();

    // Create final inspection with a lot
    service::create_final_inspection(
        &pool,
        &tenant,
        &CreateFinalInspectionRequest {
            wo_id: Uuid::new_v4(),
            lot_id: Some(lot_id),
            plan_id: None,
            part_id: Some(Uuid::new_v4()),
            part_revision: Some("A".to_string()),
            inspector_id: None,
            result: Some("pass".to_string()),
            notes: None,
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    let by_lot = service::list_inspections_by_lot(&pool, &tenant, lot_id)
        .await
        .unwrap();
    assert_eq!(by_lot.len(), 1);
    assert_eq!(by_lot[0].lot_id, Some(lot_id));
}

// ============================================================================
// Query by part_rev returns all inspection types
// ============================================================================

#[tokio::test]
#[serial]
async fn query_by_part_rev_includes_all_types() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();
    let part_id = Uuid::new_v4();

    // Receiving inspection
    service::create_receiving_inspection(
        &pool,
        &tenant,
        &CreateReceivingInspectionRequest {
            plan_id: None,
            receipt_id: Some(Uuid::new_v4()),
            lot_id: None,
            part_id: Some(part_id),
            part_revision: Some("C".to_string()),
            inspector_id: None,
            result: Some("pass".to_string()),
            notes: None,
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    // In-process inspection
    service::create_in_process_inspection(
        &pool,
        &tenant,
        &CreateInProcessInspectionRequest {
            wo_id: Uuid::new_v4(),
            op_instance_id: Uuid::new_v4(),
            plan_id: None,
            lot_id: None,
            part_id: Some(part_id),
            part_revision: Some("C".to_string()),
            inspector_id: None,
            result: Some("pass".to_string()),
            notes: None,
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    // Final inspection
    service::create_final_inspection(
        &pool,
        &tenant,
        &CreateFinalInspectionRequest {
            wo_id: Uuid::new_v4(),
            lot_id: Some(Uuid::new_v4()),
            plan_id: None,
            part_id: Some(part_id),
            part_revision: Some("C".to_string()),
            inspector_id: None,
            result: Some("pass".to_string()),
            notes: None,
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    let results =
        service::list_inspections_by_part_rev(&pool, &tenant, part_id, Some("C"))
            .await
            .unwrap();
    assert_eq!(results.len(), 3);
}

// ============================================================================
// Events emitted for in-process and final inspections
// ============================================================================

#[tokio::test]
#[serial]
async fn events_emitted_for_in_process_and_final() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    // In-process
    service::create_in_process_inspection(
        &pool,
        &tenant,
        &CreateInProcessInspectionRequest {
            wo_id: Uuid::new_v4(),
            op_instance_id: Uuid::new_v4(),
            plan_id: None,
            lot_id: None,
            part_id: None,
            part_revision: None,
            inspector_id: None,
            result: None,
            notes: None,
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    // Final
    service::create_final_inspection(
        &pool,
        &tenant,
        &CreateFinalInspectionRequest {
            wo_id: Uuid::new_v4(),
            lot_id: None,
            plan_id: None,
            part_id: None,
            part_revision: None,
            inspector_id: None,
            result: None,
            notes: None,
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    // Verify outbox has 2 events
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
            "quality_inspection.inspection_recorded",
        ]
    );

    // Verify the in-process event has correct payload fields
    let payloads: Vec<(serde_json::Value,)> = sqlx::query_as(
        "SELECT payload FROM quality_inspection_outbox WHERE tenant_id = $1 ORDER BY created_at",
    )
    .bind(&tenant)
    .fetch_all(&pool)
    .await
    .unwrap();

    let in_process_payload = &payloads[0].0;
    assert_eq!(
        in_process_payload["payload"]["inspection_type"],
        "in_process"
    );
    assert!(in_process_payload["payload"]["wo_id"].is_string());
    assert!(in_process_payload["payload"]["op_instance_id"].is_string());
    assert_eq!(in_process_payload["source_module"], "quality-inspection");
    assert_eq!(in_process_payload["replay_safe"], true);

    let final_payload = &payloads[1].0;
    assert_eq!(final_payload["payload"]["inspection_type"], "final");
    assert!(final_payload["payload"]["wo_id"].is_string());
}

// ============================================================================
// Disposition works on in-process and final inspections
// ============================================================================

#[tokio::test]
#[serial]
async fn disposition_works_on_in_process_inspection() {
    let pool = setup_db().await;
    let wc_pool = setup_wc_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();
    let inspector = Uuid::new_v4();

    authorize_inspector(&wc_pool, &tenant, inspector).await;

    let inspection = service::create_in_process_inspection(
        &pool,
        &tenant,
        &CreateInProcessInspectionRequest {
            wo_id: Uuid::new_v4(),
            op_instance_id: Uuid::new_v4(),
            plan_id: None,
            lot_id: None,
            part_id: None,
            part_revision: None,
            inspector_id: None,
            result: Some("pending".to_string()),
            notes: None,
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    assert_eq!(inspection.disposition, "pending");

    // Hold
    let held = service::hold_inspection(
        &pool,
        &wc_pool,
        &tenant,
        inspection.id,
        Some(inspector),
        Some("Dimensional out of tolerance"),
        &corr,
        None,
    )
    .await
    .unwrap();
    assert_eq!(held.disposition, "held");

    // Reject
    let rejected = service::reject_inspection(
        &pool,
        &wc_pool,
        &tenant,
        inspection.id,
        Some(inspector),
        Some("Scrap — OOT beyond rework threshold"),
        &corr,
        None,
    )
    .await
    .unwrap();
    assert_eq!(rejected.disposition, "rejected");
}
