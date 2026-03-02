//! Integration tests for plan assignments + due computation (bd-1dcy).
//!
//! Covers:
//! 1. Calendar assignment: next_due_date computed from now + interval
//! 2. Meter assignment: next_due_meter computed from latest reading + interval
//! 3. Both assignment: both next_due fields computed
//! 4. Meter with no readings: base from zero
//! 5. Duplicate assignment rejected
//! 6. List assignments with plan/asset filters

use chrono::Utc;
use maintenance_rs::domain::assets::{AssetRepo, CreateAssetRequest};
use maintenance_rs::domain::meters::{
    CreateMeterTypeRequest, MeterReadingRepo, MeterTypeRepo, RecordReadingRequest,
};
use maintenance_rs::domain::plans::{
    AssignPlanRequest, AssignmentRepo, CreatePlanRequest, ListAssignmentsQuery, PlanRepo,
};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://maintenance_user:maintenance_pass@localhost:5452/maintenance_db".to_string()
    });

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to maintenance test DB");

    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run maintenance migrations");

    pool
}

fn unique_tenant() -> String {
    format!("asn-test-{}", Uuid::new_v4().simple())
}

async fn create_test_asset(pool: &sqlx::PgPool, tid: &str, tag: &str) -> Uuid {
    let asset = AssetRepo::create(
        pool,
        &CreateAssetRequest {
            tenant_id: tid.to_string(),
            asset_tag: tag.to_string(),
            name: format!("Test Asset {}", tag),
            description: None,
            asset_type: "vehicle".into(),
            location: None,
            department: None,
            responsible_person: None,
            serial_number: None,
            fixed_asset_ref: None,
            metadata: None,
        },
    )
    .await
    .unwrap();
    asset.id
}

async fn create_test_meter(pool: &sqlx::PgPool, tid: &str, name: &str) -> Uuid {
    let meter = MeterTypeRepo::create(
        pool,
        &CreateMeterTypeRequest {
            tenant_id: tid.to_string(),
            name: name.to_string(),
            unit_label: "miles".into(),
            rollover_value: None,
        },
    )
    .await
    .unwrap();
    meter.id
}

// ============================================================================
// 1. Assign calendar plan — next_due_date computed
// ============================================================================

#[tokio::test]
#[serial]
async fn test_assign_calendar_plan_computes_next_due_date() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let asset_id = create_test_asset(&pool, &tid, "CAL-ASN-001").await;

    let plan = PlanRepo::create(
        &pool,
        &CreatePlanRequest {
            tenant_id: tid.clone(),
            name: "Monthly Check".into(),
            description: None,
            asset_type_filter: None,
            schedule_type: "calendar".into(),
            calendar_interval_days: Some(30),
            meter_type_id: None,
            meter_interval: None,
            priority: None,
            estimated_duration_minutes: None,
            estimated_cost_minor: None,
            task_checklist: None,
        },
    )
    .await
    .unwrap();

    let assignment = AssignmentRepo::assign(
        &pool,
        plan.id,
        &AssignPlanRequest {
            tenant_id: tid.clone(),
            asset_id,
        },
    )
    .await
    .unwrap();

    let expected_date = (Utc::now() + chrono::Duration::days(30)).date_naive();
    assert_eq!(assignment.next_due_date, Some(expected_date));
    assert_eq!(assignment.next_due_meter, None);
    assert_eq!(assignment.state, "active");
}

// ============================================================================
// 2. Assign meter plan — next_due_meter computed from reading
// ============================================================================

#[tokio::test]
#[serial]
async fn test_assign_meter_plan_computes_next_due_meter() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let asset_id = create_test_asset(&pool, &tid, "MTR-ASN-001").await;
    let meter_id = create_test_meter(&pool, &tid, "Odometer").await;

    MeterReadingRepo::record(
        &pool,
        asset_id,
        &RecordReadingRequest {
            tenant_id: tid.clone(),
            meter_type_id: meter_id,
            reading_value: 50_000,
            recorded_at: None,
            recorded_by: None,
        },
    )
    .await
    .unwrap();

    let plan = PlanRepo::create(
        &pool,
        &CreatePlanRequest {
            tenant_id: tid.clone(),
            name: "Tire Rotation".into(),
            description: None,
            asset_type_filter: None,
            schedule_type: "meter".into(),
            calendar_interval_days: None,
            meter_type_id: Some(meter_id),
            meter_interval: Some(5000),
            priority: None,
            estimated_duration_minutes: None,
            estimated_cost_minor: None,
            task_checklist: None,
        },
    )
    .await
    .unwrap();

    let assignment = AssignmentRepo::assign(
        &pool,
        plan.id,
        &AssignPlanRequest {
            tenant_id: tid.clone(),
            asset_id,
        },
    )
    .await
    .unwrap();

    assert_eq!(assignment.next_due_date, None);
    assert_eq!(assignment.next_due_meter, Some(55_000));
}

// ============================================================================
// 3. Assign "both" plan — both fields computed
// ============================================================================

#[tokio::test]
#[serial]
async fn test_assign_both_plan_computes_both_due_fields() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let asset_id = create_test_asset(&pool, &tid, "BOTH-ASN-001").await;
    let meter_id = create_test_meter(&pool, &tid, "Engine Hours").await;

    MeterReadingRepo::record(
        &pool,
        asset_id,
        &RecordReadingRequest {
            tenant_id: tid.clone(),
            meter_type_id: meter_id,
            reading_value: 1000,
            recorded_at: None,
            recorded_by: None,
        },
    )
    .await
    .unwrap();

    let plan = PlanRepo::create(
        &pool,
        &CreatePlanRequest {
            tenant_id: tid.clone(),
            name: "Full Service".into(),
            description: None,
            asset_type_filter: None,
            schedule_type: "both".into(),
            calendar_interval_days: Some(90),
            meter_type_id: Some(meter_id),
            meter_interval: Some(250),
            priority: None,
            estimated_duration_minutes: None,
            estimated_cost_minor: None,
            task_checklist: None,
        },
    )
    .await
    .unwrap();

    let assignment = AssignmentRepo::assign(
        &pool,
        plan.id,
        &AssignPlanRequest {
            tenant_id: tid.clone(),
            asset_id,
        },
    )
    .await
    .unwrap();

    let expected_date = (Utc::now() + chrono::Duration::days(90)).date_naive();
    assert_eq!(assignment.next_due_date, Some(expected_date));
    assert_eq!(assignment.next_due_meter, Some(1250));
}

// ============================================================================
// 4. Meter plan with no readings — base from zero
// ============================================================================

#[tokio::test]
#[serial]
async fn test_assign_meter_plan_no_readings_base_zero() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let asset_id = create_test_asset(&pool, &tid, "ZERO-ASN-001").await;
    let meter_id = create_test_meter(&pool, &tid, "Odometer").await;

    let plan = PlanRepo::create(
        &pool,
        &CreatePlanRequest {
            tenant_id: tid.clone(),
            name: "First Service".into(),
            description: None,
            asset_type_filter: None,
            schedule_type: "meter".into(),
            calendar_interval_days: None,
            meter_type_id: Some(meter_id),
            meter_interval: Some(5000),
            priority: None,
            estimated_duration_minutes: None,
            estimated_cost_minor: None,
            task_checklist: None,
        },
    )
    .await
    .unwrap();

    let assignment = AssignmentRepo::assign(
        &pool,
        plan.id,
        &AssignPlanRequest {
            tenant_id: tid.clone(),
            asset_id,
        },
    )
    .await
    .unwrap();

    assert_eq!(assignment.next_due_meter, Some(5000));
}

// ============================================================================
// 5. Duplicate assignment rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn test_duplicate_assignment_rejected() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let asset_id = create_test_asset(&pool, &tid, "DUP-ASN-001").await;

    let plan = PlanRepo::create(
        &pool,
        &CreatePlanRequest {
            tenant_id: tid.clone(),
            name: "Weekly Check".into(),
            description: None,
            asset_type_filter: None,
            schedule_type: "calendar".into(),
            calendar_interval_days: Some(7),
            meter_type_id: None,
            meter_interval: None,
            priority: None,
            estimated_duration_minutes: None,
            estimated_cost_minor: None,
            task_checklist: None,
        },
    )
    .await
    .unwrap();

    AssignmentRepo::assign(
        &pool,
        plan.id,
        &AssignPlanRequest {
            tenant_id: tid.clone(),
            asset_id,
        },
    )
    .await
    .unwrap();

    let err = AssignmentRepo::assign(
        &pool,
        plan.id,
        &AssignPlanRequest {
            tenant_id: tid,
            asset_id,
        },
    )
    .await
    .unwrap_err();

    assert!(
        matches!(
            err,
            maintenance_rs::domain::plans::PlanError::DuplicateAssignment
        ),
        "expected DuplicateAssignment, got: {:?}",
        err
    );
}

// ============================================================================
// 6. List assignments with filters
// ============================================================================

#[tokio::test]
#[serial]
async fn test_list_assignments_with_filters() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let asset_a = create_test_asset(&pool, &tid, "FLTR-A").await;
    let asset_b = create_test_asset(&pool, &tid, "FLTR-B").await;

    let plan = PlanRepo::create(
        &pool,
        &CreatePlanRequest {
            tenant_id: tid.clone(),
            name: "Filter Test Plan".into(),
            description: None,
            asset_type_filter: None,
            schedule_type: "calendar".into(),
            calendar_interval_days: Some(30),
            meter_type_id: None,
            meter_interval: None,
            priority: None,
            estimated_duration_minutes: None,
            estimated_cost_minor: None,
            task_checklist: None,
        },
    )
    .await
    .unwrap();

    AssignmentRepo::assign(
        &pool,
        plan.id,
        &AssignPlanRequest {
            tenant_id: tid.clone(),
            asset_id: asset_a,
        },
    )
    .await
    .unwrap();

    AssignmentRepo::assign(
        &pool,
        plan.id,
        &AssignPlanRequest {
            tenant_id: tid.clone(),
            asset_id: asset_b,
        },
    )
    .await
    .unwrap();

    let all = AssignmentRepo::list(
        &pool,
        &ListAssignmentsQuery {
            tenant_id: tid.clone(),
            plan_id: None,
            asset_id: None,
            limit: None,
            offset: None,
        },
    )
    .await
    .unwrap();
    assert!(all.len() >= 2);

    let by_plan = AssignmentRepo::list(
        &pool,
        &ListAssignmentsQuery {
            tenant_id: tid.clone(),
            plan_id: Some(plan.id),
            asset_id: None,
            limit: None,
            offset: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(by_plan.len(), 2);

    let by_asset = AssignmentRepo::list(
        &pool,
        &ListAssignmentsQuery {
            tenant_id: tid,
            plan_id: None,
            asset_id: Some(asset_a),
            limit: None,
            offset: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(by_asset.len(), 1);
}
