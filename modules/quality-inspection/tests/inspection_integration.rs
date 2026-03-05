use quality_inspection_rs::domain::models::*;
use quality_inspection_rs::domain::service;
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://quality_inspection_user:quality_inspection_pass@localhost:5459/quality_inspection_db".to_string()
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

fn unique_tenant() -> String {
    format!("test-tenant-{}", Uuid::new_v4())
}

// ============================================================================
// Create inspection plan with characteristics
// ============================================================================

#[tokio::test]
#[serial]
async fn create_inspection_plan_with_characteristics() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let part_id = Uuid::new_v4();
    let corr = Uuid::new_v4().to_string();

    let plan = service::create_inspection_plan(
        &pool,
        &tenant,
        &CreateInspectionPlanRequest {
            part_id,
            plan_name: "Receiving Plan for Fastener".to_string(),
            revision: Some("B".to_string()),
            characteristics: vec![
                Characteristic {
                    name: "Diameter".to_string(),
                    characteristic_type: "dimensional".to_string(),
                    nominal: Some(10.0),
                    tolerance_low: Some(9.95),
                    tolerance_high: Some(10.05),
                    uom: Some("mm".to_string()),
                },
                Characteristic {
                    name: "Surface finish".to_string(),
                    characteristic_type: "visual".to_string(),
                    nominal: None,
                    tolerance_low: None,
                    tolerance_high: None,
                    uom: None,
                },
            ],
            sampling_method: Some("aql".to_string()),
            sample_size: Some(13),
        },
        &corr,
        None,
    )
    .await
    .expect("create_inspection_plan");

    assert_eq!(plan.tenant_id, tenant);
    assert_eq!(plan.part_id, part_id);
    assert_eq!(plan.plan_name, "Receiving Plan for Fastener");
    assert_eq!(plan.revision, "B");
    assert_eq!(plan.status, "draft");
    assert_eq!(plan.sampling_method, "aql");
    assert_eq!(plan.sample_size, Some(13));

    let chars: Vec<Characteristic> =
        serde_json::from_value(plan.characteristics).expect("parse characteristics");
    assert_eq!(chars.len(), 2);
    assert_eq!(chars[0].name, "Diameter");
    assert!((chars[0].nominal.unwrap() - 10.0).abs() < f64::EPSILON);
}

// ============================================================================
// Activate plan
// ============================================================================

#[tokio::test]
#[serial]
async fn activate_inspection_plan() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let plan = service::create_inspection_plan(
        &pool,
        &tenant,
        &CreateInspectionPlanRequest {
            part_id: Uuid::new_v4(),
            plan_name: "Plan A".to_string(),
            revision: None,
            characteristics: vec![],
            sampling_method: None,
            sample_size: None,
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    assert_eq!(plan.status, "draft");

    let activated = service::activate_plan(&pool, &tenant, plan.id)
        .await
        .expect("activate_plan");
    assert_eq!(activated.status, "active");
}

// ============================================================================
// Create receiving inspection + query by receipt
// ============================================================================

#[tokio::test]
#[serial]
async fn create_receiving_inspection_and_query_by_receipt() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();
    let receipt_id = Uuid::new_v4();
    let part_id = Uuid::new_v4();

    let inspection = service::create_receiving_inspection(
        &pool,
        &tenant,
        &CreateReceivingInspectionRequest {
            plan_id: None,
            receipt_id: Some(receipt_id),
            lot_id: None,
            part_id: Some(part_id),
            part_revision: Some("C".to_string()),
            inspector_id: None,
            result: Some("pass".to_string()),
            notes: Some("All dimensions within spec".to_string()),
        },
        &corr,
        None,
    )
    .await
    .expect("create_receiving_inspection");

    assert_eq!(inspection.tenant_id, tenant);
    assert_eq!(inspection.inspection_type, "receiving");
    assert_eq!(inspection.result, "pass");
    assert_eq!(inspection.receipt_id, Some(receipt_id));
    assert_eq!(inspection.part_id, Some(part_id));
    assert_eq!(inspection.part_revision.as_deref(), Some("C"));
    assert!(inspection.inspected_at.is_some());

    // Query by receipt
    let by_receipt = service::list_inspections_by_receipt(&pool, &tenant, receipt_id)
        .await
        .expect("list_by_receipt");
    assert_eq!(by_receipt.len(), 1);
    assert_eq!(by_receipt[0].id, inspection.id);
}

// ============================================================================
// Query by part revision
// ============================================================================

#[tokio::test]
#[serial]
async fn query_inspections_by_part_revision() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();
    let part_id = Uuid::new_v4();

    // Create two inspections for same part, different revisions
    service::create_receiving_inspection(
        &pool,
        &tenant,
        &CreateReceivingInspectionRequest {
            plan_id: None,
            receipt_id: Some(Uuid::new_v4()),
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

    service::create_receiving_inspection(
        &pool,
        &tenant,
        &CreateReceivingInspectionRequest {
            plan_id: None,
            receipt_id: Some(Uuid::new_v4()),
            lot_id: None,
            part_id: Some(part_id),
            part_revision: Some("B".to_string()),
            inspector_id: None,
            result: Some("fail".to_string()),
            notes: None,
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    // Query all for part
    let all = service::list_inspections_by_part_rev(&pool, &tenant, part_id, None)
        .await
        .unwrap();
    assert_eq!(all.len(), 2);

    // Query specific revision
    let rev_a = service::list_inspections_by_part_rev(&pool, &tenant, part_id, Some("A"))
        .await
        .unwrap();
    assert_eq!(rev_a.len(), 1);
    assert_eq!(rev_a[0].result, "pass");

    let rev_b = service::list_inspections_by_part_rev(&pool, &tenant, part_id, Some("B"))
        .await
        .unwrap();
    assert_eq!(rev_b.len(), 1);
    assert_eq!(rev_b[0].result, "fail");
}

// ============================================================================
// Events emitted to outbox
// ============================================================================

#[tokio::test]
#[serial]
async fn events_emitted_to_outbox() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    // Create plan → 1 event
    let plan = service::create_inspection_plan(
        &pool,
        &tenant,
        &CreateInspectionPlanRequest {
            part_id: Uuid::new_v4(),
            plan_name: "Outbox test plan".to_string(),
            revision: None,
            characteristics: vec![],
            sampling_method: None,
            sample_size: None,
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    // Create inspection → 1 event
    service::create_receiving_inspection(
        &pool,
        &tenant,
        &CreateReceivingInspectionRequest {
            plan_id: Some(plan.id),
            receipt_id: Some(Uuid::new_v4()),
            lot_id: None,
            part_id: Some(Uuid::new_v4()),
            part_revision: Some("A".to_string()),
            inspector_id: None,
            result: None,
            notes: None,
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    // Verify outbox
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
            "quality_inspection.plan_created",
            "quality_inspection.inspection_recorded"
        ]
    );

    // Verify envelope metadata
    let payload: (serde_json::Value,) = sqlx::query_as(
        "SELECT payload FROM quality_inspection_outbox WHERE tenant_id = $1 AND event_type = 'quality_inspection.plan_created'",
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(payload.0["source_module"], "quality-inspection");
    assert_eq!(payload.0["replay_safe"], true);
    assert!(payload.0["event_id"].is_string());
    assert!(payload.0["correlation_id"].is_string());
    assert_eq!(payload.0["mutation_class"], "DATA_MUTATION");
}

// ============================================================================
// Tenant isolation
// ============================================================================

#[tokio::test]
#[serial]
async fn tenant_isolation() {
    let pool = setup_db().await;
    let tenant_a = unique_tenant();
    let tenant_b = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let plan = service::create_inspection_plan(
        &pool,
        &tenant_a,
        &CreateInspectionPlanRequest {
            part_id: Uuid::new_v4(),
            plan_name: "Secret plan".to_string(),
            revision: None,
            characteristics: vec![],
            sampling_method: None,
            sample_size: None,
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    // Tenant B cannot see it
    let result = service::get_inspection_plan(&pool, &tenant_b, plan.id).await;
    assert!(result.is_err());

    // Tenant B cannot activate it
    let result = service::activate_plan(&pool, &tenant_b, plan.id).await;
    assert!(result.is_err());
}
