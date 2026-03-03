//! Integration tests for auto-create work orders from due plans (bd-22f2).
//!
//! Covers:
//! 1. Auto-create disabled (default): tick emits event but creates no WO
//! 2. Auto-create enabled, no approval gate: WO created with status 'scheduled'
//! 3. Auto-create enabled + approvals_required: WO status is 'awaiting_approval'
//! 4. Idempotency: second tick does not create a duplicate WO
//! 5. WO inherits plan priority and checklist

use chrono::{Duration, Utc};
use maintenance_rs::domain::assets::{AssetRepo, CreateAssetRequest};
use maintenance_rs::domain::plans::{
    AssignPlanRequest, AssignmentRepo, CreatePlanRequest, PlanRepo,
};
use maintenance_rs::domain::scheduler::evaluate_due;
use maintenance_rs::domain::tenant_config::TenantConfigRepo;
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

fn tid() -> String {
    format!("auto-test-{}", Uuid::new_v4().simple())
}

async fn mk_asset(pool: &sqlx::PgPool, tid: &str, tag: &str) -> Uuid {
    AssetRepo::create(
        pool,
        &CreateAssetRequest {
            tenant_id: tid.into(),
            asset_tag: tag.into(),
            name: format!("Asset {tag}"),
            description: None,
            asset_type: "vehicle".into(),
            location: None,
            department: None,
            responsible_person: None,
            serial_number: None,
            fixed_asset_ref: None,
            metadata: None,
            maintenance_schedule: None,
            idempotency_key: None,
        },
    )
    .await
    .unwrap()
    .id
}

async fn mk_cal_plan(pool: &sqlx::PgPool, tid: &str, name: &str, days: i32) -> Uuid {
    PlanRepo::create(
        pool,
        &CreatePlanRequest {
            tenant_id: tid.into(),
            name: name.into(),
            description: None,
            asset_type_filter: None,
            schedule_type: "calendar".into(),
            calendar_interval_days: Some(days),
            meter_type_id: None,
            meter_interval: None,
            priority: Some("high".into()),
            estimated_duration_minutes: None,
            estimated_cost_minor: None,
            task_checklist: None,
        },
    )
    .await
    .unwrap()
    .id
}

async fn assign(pool: &sqlx::PgPool, plan_id: Uuid, tid: &str, asset_id: Uuid) -> Uuid {
    AssignmentRepo::assign(
        pool,
        plan_id,
        &AssignPlanRequest {
            tenant_id: tid.into(),
            asset_id,
        },
    )
    .await
    .unwrap()
    .id
}

async fn backdate(pool: &sqlx::PgPool, assignment_id: Uuid, days_ago: i64) {
    let d = (Utc::now() - Duration::days(days_ago)).date_naive();
    sqlx::query("UPDATE maintenance_plan_assignments SET next_due_date = $1 WHERE id = $2")
        .bind(d)
        .bind(assignment_id)
        .execute(pool)
        .await
        .unwrap();
}

async fn count_wos_for_assignment(pool: &sqlx::PgPool, assignment_id: Uuid) -> i64 {
    let r: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM work_orders WHERE plan_assignment_id = $1")
            .bind(assignment_id)
            .fetch_one(pool)
            .await
            .unwrap();
    r.0
}

// ============================================================================
// 1. Auto-create disabled (default): no WO created
// ============================================================================

#[tokio::test]
#[serial]
async fn test_auto_create_disabled_no_wo() {
    let pool = setup_db().await;
    let t = tid();
    let asset = mk_asset(&pool, &t, "NO-AUTO-001").await;
    let plan = mk_cal_plan(&pool, &t, "No Auto Plan", 1).await;
    let asn = assign(&pool, plan, &t, asset).await;
    backdate(&pool, asn, 1).await;

    let r = evaluate_due(&pool).await.unwrap();
    assert_eq!(r.events_emitted, 1, "event should still be emitted");
    assert_eq!(
        r.work_orders_created, 0,
        "no WO when auto_create_on_due=false"
    );
    assert_eq!(count_wos_for_assignment(&pool, asn).await, 0);
}

// ============================================================================
// 2. Auto-create enabled, no approval gate: WO status = 'scheduled'
// ============================================================================

#[tokio::test]
#[serial]
async fn test_auto_create_scheduled() {
    let pool = setup_db().await;
    let t = tid();

    TenantConfigRepo::upsert(&pool, &t, true, false)
        .await
        .unwrap();

    let asset = mk_asset(&pool, &t, "AUTO-001").await;
    let plan = mk_cal_plan(&pool, &t, "Auto Plan", 1).await;
    let asn = assign(&pool, plan, &t, asset).await;
    backdate(&pool, asn, 1).await;

    let r = evaluate_due(&pool).await.unwrap();
    assert_eq!(r.events_emitted, 1);
    assert_eq!(r.work_orders_created, 1);
    assert_eq!(count_wos_for_assignment(&pool, asn).await, 1);

    // Verify WO status and attributes
    let wo: (String, String, String, String) = sqlx::query_as(
        "SELECT status, wo_type, priority, title FROM work_orders WHERE plan_assignment_id = $1",
    )
    .bind(asn)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(
        wo.0, "scheduled",
        "WO should be scheduled when approvals_required=false"
    );
    assert_eq!(wo.1, "preventive", "auto-created WO should be preventive");
    assert_eq!(wo.2, "high", "WO should inherit plan priority");
    assert!(wo.3.contains("Auto Plan"), "title should contain plan name");
}

// ============================================================================
// 3. Auto-create + approvals_required: WO status = 'awaiting_approval'
// ============================================================================

#[tokio::test]
#[serial]
async fn test_auto_create_awaiting_approval() {
    let pool = setup_db().await;
    let t = tid();

    TenantConfigRepo::upsert(&pool, &t, true, true)
        .await
        .unwrap();

    let asset = mk_asset(&pool, &t, "APPR-001").await;
    let plan = mk_cal_plan(&pool, &t, "Approval Plan", 1).await;
    let asn = assign(&pool, plan, &t, asset).await;
    backdate(&pool, asn, 1).await;

    let r = evaluate_due(&pool).await.unwrap();
    assert_eq!(r.events_emitted, 1);
    assert_eq!(r.work_orders_created, 1);

    let status: (String,) =
        sqlx::query_as("SELECT status FROM work_orders WHERE plan_assignment_id = $1")
            .bind(asn)
            .fetch_one(&pool)
            .await
            .unwrap();

    assert_eq!(status.0, "awaiting_approval");
}

// ============================================================================
// 4. Idempotency: second tick does not create duplicate WO
// ============================================================================

#[tokio::test]
#[serial]
async fn test_auto_create_idempotent() {
    let pool = setup_db().await;
    let t = tid();

    TenantConfigRepo::upsert(&pool, &t, true, false)
        .await
        .unwrap();

    let asset = mk_asset(&pool, &t, "IDEM-001").await;
    let plan = mk_cal_plan(&pool, &t, "Idempotent Plan", 1).await;
    let asn = assign(&pool, plan, &t, asset).await;
    backdate(&pool, asn, 1).await;

    let r1 = evaluate_due(&pool).await.unwrap();
    assert_eq!(r1.work_orders_created, 1);

    let r2 = evaluate_due(&pool).await.unwrap();
    assert_eq!(r2.events_emitted, 0, "second tick should not re-emit");
    assert_eq!(
        r2.work_orders_created, 0,
        "second tick should not create another WO"
    );

    assert_eq!(
        count_wos_for_assignment(&pool, asn).await,
        1,
        "still exactly 1 WO"
    );
}

// ============================================================================
// 5. WO inherits plan checklist
// ============================================================================

#[tokio::test]
#[serial]
async fn test_auto_create_inherits_checklist() {
    let pool = setup_db().await;
    let t = tid();

    TenantConfigRepo::upsert(&pool, &t, true, false)
        .await
        .unwrap();

    let checklist = serde_json::json!(["Check oil", "Inspect tires", "Test brakes"]);
    let plan = PlanRepo::create(
        &pool,
        &CreatePlanRequest {
            tenant_id: t.clone(),
            name: "Checklist Plan".into(),
            description: None,
            asset_type_filter: None,
            schedule_type: "calendar".into(),
            calendar_interval_days: Some(7),
            meter_type_id: None,
            meter_interval: None,
            priority: Some("critical".into()),
            estimated_duration_minutes: None,
            estimated_cost_minor: None,
            task_checklist: Some(checklist.clone()),
        },
    )
    .await
    .unwrap();

    let asset = mk_asset(&pool, &t, "CL-001").await;
    let asn = assign(&pool, plan.id, &t, asset).await;
    backdate(&pool, asn, 1).await;

    evaluate_due(&pool).await.unwrap();

    let wo_checklist: (Option<serde_json::Value>,) =
        sqlx::query_as("SELECT checklist FROM work_orders WHERE plan_assignment_id = $1")
            .bind(asn)
            .fetch_one(&pool)
            .await
            .unwrap();

    assert_eq!(
        wo_checklist.0.unwrap(),
        checklist,
        "WO should inherit plan checklist"
    );
}
