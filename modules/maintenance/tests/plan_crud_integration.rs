//! Integration tests for maintenance plan CRUD (bd-1dcy).
//!
//! Covers:
//! 1. Plan CRUD: create calendar/meter/both plans, get, list, update
//! 2. Validation: missing required fields per schedule_type
//! 3. Tenant isolation for plans

use maintenance_rs::domain::meters::{CreateMeterTypeRequest, MeterTypeRepo};
use maintenance_rs::domain::plans::{
    CreatePlanRequest, ListPlansQuery, PlanRepo, UpdatePlanRequest,
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
    format!("plan-test-{}", Uuid::new_v4().simple())
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
// 1. Create calendar plan
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_calendar_plan() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let plan = PlanRepo::create(
        &pool,
        &CreatePlanRequest {
            tenant_id: tid.clone(),
            name: "Oil Change".into(),
            description: Some("Regular oil change".into()),
            asset_type_filter: Some("vehicle".into()),
            schedule_type: "calendar".into(),
            calendar_interval_days: Some(90),
            meter_type_id: None,
            meter_interval: None,
            priority: Some("medium".into()),
            estimated_duration_minutes: Some(60),
            estimated_cost_minor: Some(5000),
            task_checklist: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(plan.name, "Oil Change");
    assert_eq!(plan.calendar_interval_days, Some(90));
    assert!(plan.is_active);

    let fetched = PlanRepo::find_by_id(&pool, plan.id, &tid)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fetched.id, plan.id);
}

// ============================================================================
// 2. Create meter plan
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_meter_plan() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let meter_id = create_test_meter(&pool, &tid, "Odometer").await;

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

    assert_eq!(plan.meter_type_id, Some(meter_id));
    assert_eq!(plan.meter_interval, Some(5000));
}

// ============================================================================
// 3. Create both plan
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_both_plan() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let meter_id = create_test_meter(&pool, &tid, "Engine Hours").await;

    let plan = PlanRepo::create(
        &pool,
        &CreatePlanRequest {
            tenant_id: tid.clone(),
            name: "Full Inspection".into(),
            description: None,
            asset_type_filter: None,
            schedule_type: "both".into(),
            calendar_interval_days: Some(180),
            meter_type_id: Some(meter_id),
            meter_interval: Some(500),
            priority: Some("high".into()),
            estimated_duration_minutes: Some(240),
            estimated_cost_minor: None,
            task_checklist: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(plan.calendar_interval_days, Some(180));
    assert_eq!(plan.meter_interval, Some(500));
}

// ============================================================================
// 4. Validation: calendar without interval_days
// ============================================================================

#[tokio::test]
#[serial]
async fn test_calendar_plan_requires_interval_days() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let err = PlanRepo::create(
        &pool,
        &CreatePlanRequest {
            tenant_id: tid,
            name: "Bad Plan".into(),
            description: None,
            asset_type_filter: None,
            schedule_type: "calendar".into(),
            calendar_interval_days: None,
            meter_type_id: None,
            meter_interval: None,
            priority: None,
            estimated_duration_minutes: None,
            estimated_cost_minor: None,
            task_checklist: None,
        },
    )
    .await
    .unwrap_err();

    assert!(
        matches!(err, maintenance_rs::domain::plans::PlanError::Validation(_)),
        "expected Validation, got: {:?}",
        err
    );
}

// ============================================================================
// 5. Validation: meter without meter_type_id
// ============================================================================

#[tokio::test]
#[serial]
async fn test_meter_plan_requires_meter_fields() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let err = PlanRepo::create(
        &pool,
        &CreatePlanRequest {
            tenant_id: tid,
            name: "Bad Meter Plan".into(),
            description: None,
            asset_type_filter: None,
            schedule_type: "meter".into(),
            calendar_interval_days: None,
            meter_type_id: None,
            meter_interval: None,
            priority: None,
            estimated_duration_minutes: None,
            estimated_cost_minor: None,
            task_checklist: None,
        },
    )
    .await
    .unwrap_err();

    assert!(
        matches!(err, maintenance_rs::domain::plans::PlanError::Validation(_)),
        "expected Validation, got: {:?}",
        err
    );
}

// ============================================================================
// 6. List and update plans
// ============================================================================

#[tokio::test]
#[serial]
async fn test_list_and_update_plans() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    PlanRepo::create(
        &pool,
        &CreatePlanRequest {
            tenant_id: tid.clone(),
            name: "Plan A".into(),
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

    let plan_b = PlanRepo::create(
        &pool,
        &CreatePlanRequest {
            tenant_id: tid.clone(),
            name: "Plan B".into(),
            description: None,
            asset_type_filter: None,
            schedule_type: "calendar".into(),
            calendar_interval_days: Some(60),
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

    let plans = PlanRepo::list(
        &pool,
        &ListPlansQuery {
            tenant_id: tid.clone(),
            is_active: None,
            limit: None,
            offset: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(plans.len(), 2);

    let updated = PlanRepo::update(
        &pool,
        plan_b.id,
        &tid,
        &UpdatePlanRequest {
            name: Some("Plan B Revised".into()),
            description: None,
            priority: Some("high".into()),
            estimated_duration_minutes: None,
            estimated_cost_minor: None,
            task_checklist: None,
            is_active: Some(false),
        },
    )
    .await
    .unwrap();

    assert_eq!(updated.name, "Plan B Revised");
    assert!(!updated.is_active);

    let active = PlanRepo::list(
        &pool,
        &ListPlansQuery {
            tenant_id: tid,
            is_active: Some(true),
            limit: None,
            offset: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(active.len(), 1);
}

// ============================================================================
// 7. Tenant isolation — cross-tenant plan access
// ============================================================================

#[tokio::test]
#[serial]
async fn test_tenant_isolation_plans() {
    let pool = setup_db().await;
    let tid_a = unique_tenant();
    let tid_b = unique_tenant();

    let plan = PlanRepo::create(
        &pool,
        &CreatePlanRequest {
            tenant_id: tid_a.clone(),
            name: "Tenant A Plan".into(),
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

    let result = PlanRepo::find_by_id(&pool, plan.id, &tid_b).await.unwrap();
    assert!(result.is_none());

    let list = PlanRepo::list(
        &pool,
        &ListPlansQuery {
            tenant_id: tid_b.clone(),
            is_active: None,
            limit: None,
            offset: None,
        },
    )
    .await
    .unwrap();
    assert!(list.is_empty());
}
