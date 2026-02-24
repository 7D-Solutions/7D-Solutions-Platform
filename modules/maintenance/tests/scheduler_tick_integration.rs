//! Integration tests for the maintenance scheduler tick (bd-1wuh).
//!
//! Covers:
//! 1. Calendar-due: tick emits maintenance.plan.due, sets due_notified_at
//! 2. Meter-due: tick emits event when reading exceeds threshold
//! 3. Both-schedule: calendar trigger fires, event payload verified
//! 4. Idempotency: second tick does not re-emit
//! 5. Inactive plan skipped
//! 6. Paused assignment skipped

use chrono::{Duration, Utc};
use maintenance_rs::domain::assets::{AssetRepo, CreateAssetRequest};
use maintenance_rs::domain::meters::{
    CreateMeterTypeRequest, MeterReadingRepo, MeterTypeRepo, RecordReadingRequest,
};
use maintenance_rs::domain::plans::{
    AssignPlanRequest, AssignmentRepo, CreatePlanRequest, PlanRepo,
};
use maintenance_rs::domain::scheduler::evaluate_due;
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://maintenance_user:maintenance_pass@localhost:5452/maintenance_db"
            .to_string()
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
    format!("sched-test-{}", Uuid::new_v4().simple())
}

async fn mk_asset(pool: &sqlx::PgPool, tid: &str, tag: &str) -> Uuid {
    AssetRepo::create(pool, &CreateAssetRequest {
        tenant_id: tid.into(), asset_tag: tag.into(), name: format!("Asset {tag}"),
        description: None, asset_type: "vehicle".into(), location: None,
        department: None, responsible_person: None, serial_number: None,
        fixed_asset_ref: None, metadata: None,
    }).await.unwrap().id
}

async fn mk_meter(pool: &sqlx::PgPool, tid: &str, name: &str) -> Uuid {
    MeterTypeRepo::create(pool, &CreateMeterTypeRequest {
        tenant_id: tid.into(), name: name.into(), unit_label: "mi".into(),
        rollover_value: None,
    }).await.unwrap().id
}

async fn mk_cal_plan(pool: &sqlx::PgPool, tid: &str, name: &str, days: i32) -> Uuid {
    PlanRepo::create(pool, &CreatePlanRequest {
        tenant_id: tid.into(), name: name.into(), description: None,
        asset_type_filter: None, schedule_type: "calendar".into(),
        calendar_interval_days: Some(days), meter_type_id: None,
        meter_interval: None, priority: None, estimated_duration_minutes: None,
        estimated_cost_minor: None, task_checklist: None,
    }).await.unwrap().id
}

async fn assign(pool: &sqlx::PgPool, plan_id: Uuid, tid: &str, asset_id: Uuid) -> Uuid {
    AssignmentRepo::assign(pool, plan_id, &AssignPlanRequest {
        tenant_id: tid.into(), asset_id,
    }).await.unwrap().id
}

async fn backdate(pool: &sqlx::PgPool, assignment_id: Uuid, days_ago: i64) {
    let d = (Utc::now() - Duration::days(days_ago)).date_naive();
    sqlx::query("UPDATE maintenance_plan_assignments SET next_due_date = $1 WHERE id = $2")
        .bind(d).bind(assignment_id).execute(pool).await.unwrap();
}

async fn count_due_events(pool: &sqlx::PgPool, agg_id: &str) -> i64 {
    let r: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM events_outbox WHERE event_type = 'maintenance.plan.due' AND aggregate_id = $1",
    ).bind(agg_id).fetch_one(pool).await.unwrap();
    r.0
}

// ============================================================================
// 1. Calendar-due: next_due_date in the past triggers event
// ============================================================================

#[tokio::test]
#[serial]
async fn test_calendar_due_emits_event() {
    let pool = setup_db().await;
    let t = tid();
    let asset = mk_asset(&pool, &t, "CAL-001").await;
    let plan = mk_cal_plan(&pool, &t, "Daily Check", 1).await;
    let asn = assign(&pool, plan, &t, asset).await;
    backdate(&pool, asn, 1).await;

    let r = evaluate_due(&pool).await.unwrap();
    assert_eq!(r.evaluated, 1);
    assert_eq!(r.events_emitted, 1);
    assert_eq!(count_due_events(&pool, &asn.to_string()).await, 1);

    let updated = AssignmentRepo::find_by_id(&pool, asn, &t).await.unwrap().unwrap();
    assert!(updated.due_notified_at.is_some(), "due_notified_at should be set");
}

// ============================================================================
// 2. Meter-due: reading exceeds threshold triggers event
// ============================================================================

#[tokio::test]
#[serial]
async fn test_meter_due_emits_event() {
    let pool = setup_db().await;
    let t = tid();
    let asset = mk_asset(&pool, &t, "MTR-001").await;
    let meter = mk_meter(&pool, &t, "Odometer").await;

    MeterReadingRepo::record(&pool, asset, &RecordReadingRequest {
        tenant_id: t.clone(), meter_type_id: meter, reading_value: 50_000,
        recorded_at: None, recorded_by: None,
    }).await.unwrap();

    let plan = PlanRepo::create(&pool, &CreatePlanRequest {
        tenant_id: t.clone(), name: "Tire Rotation".into(), description: None,
        asset_type_filter: None, schedule_type: "meter".into(),
        calendar_interval_days: None, meter_type_id: Some(meter),
        meter_interval: Some(5000), priority: None,
        estimated_duration_minutes: None, estimated_cost_minor: None,
        task_checklist: None,
    }).await.unwrap();

    let asn = assign(&pool, plan.id, &t, asset).await;

    // Not yet due (reading=50k, threshold=55k)
    let r = evaluate_due(&pool).await.unwrap();
    assert_eq!(r.events_emitted, 0, "should not be due yet");

    // Record reading past threshold
    MeterReadingRepo::record(&pool, asset, &RecordReadingRequest {
        tenant_id: t.clone(), meter_type_id: meter, reading_value: 56_000,
        recorded_at: None, recorded_by: None,
    }).await.unwrap();

    let r = evaluate_due(&pool).await.unwrap();
    assert_eq!(r.evaluated, 1);
    assert_eq!(r.events_emitted, 1);
    assert_eq!(count_due_events(&pool, &asn.to_string()).await, 1);
}

// ============================================================================
// 3. Both-schedule: calendar trigger fires, event payload verified
// ============================================================================

#[tokio::test]
#[serial]
async fn test_both_schedule_calendar_trigger() {
    let pool = setup_db().await;
    let t = tid();
    let asset = mk_asset(&pool, &t, "BOTH-001").await;
    let meter = mk_meter(&pool, &t, "Engine Hours").await;

    MeterReadingRepo::record(&pool, asset, &RecordReadingRequest {
        tenant_id: t.clone(), meter_type_id: meter, reading_value: 1000,
        recorded_at: None, recorded_by: None,
    }).await.unwrap();

    let plan = PlanRepo::create(&pool, &CreatePlanRequest {
        tenant_id: t.clone(), name: "Full Service".into(), description: None,
        asset_type_filter: None, schedule_type: "both".into(),
        calendar_interval_days: Some(30), meter_type_id: Some(meter),
        meter_interval: Some(500), priority: Some("critical".into()),
        estimated_duration_minutes: None, estimated_cost_minor: None,
        task_checklist: None,
    }).await.unwrap();

    let asn = assign(&pool, plan.id, &t, asset).await;
    backdate(&pool, asn, 1).await; // calendar due, meter not yet

    let r = evaluate_due(&pool).await.unwrap();
    assert_eq!(r.events_emitted, 1);

    // Verify event payload
    let payload: serde_json::Value = sqlx::query_scalar(
        "SELECT payload FROM events_outbox WHERE event_type = 'maintenance.plan.due' AND aggregate_id = $1 ORDER BY created_at DESC LIMIT 1",
    ).bind(asn.to_string()).fetch_one(&pool).await.unwrap();

    assert_eq!(payload["plan_priority"], "critical");
    assert_eq!(payload["assignment_id"], asn.to_string());
    assert_eq!(payload["plan_id"], plan.id.to_string());
    assert_eq!(payload["asset_id"], asset.to_string());
}

// ============================================================================
// 4. Idempotency: second tick does not re-emit
// ============================================================================

#[tokio::test]
#[serial]
async fn test_idempotent_no_double_emission() {
    let pool = setup_db().await;
    let t = tid();
    let asset = mk_asset(&pool, &t, "IDEM-001").await;
    let plan = mk_cal_plan(&pool, &t, "Weekly Check", 7).await;
    let asn = assign(&pool, plan, &t, asset).await;
    backdate(&pool, asn, 10).await;

    let r1 = evaluate_due(&pool).await.unwrap();
    assert_eq!(r1.events_emitted, 1);

    let r2 = evaluate_due(&pool).await.unwrap();
    assert_eq!(r2.events_emitted, 0, "second tick must not re-emit");

    assert_eq!(count_due_events(&pool, &asn.to_string()).await, 1);
}

// ============================================================================
// 5. Inactive plan skipped
// ============================================================================

#[tokio::test]
#[serial]
async fn test_inactive_plan_skipped() {
    let pool = setup_db().await;
    let t = tid();
    let asset = mk_asset(&pool, &t, "INACT-001").await;
    let plan = mk_cal_plan(&pool, &t, "Deactivated Plan", 1).await;
    let asn = assign(&pool, plan, &t, asset).await;
    backdate(&pool, asn, 1).await;

    sqlx::query("UPDATE maintenance_plans SET is_active = false WHERE id = $1")
        .bind(plan).execute(&pool).await.unwrap();

    let r = evaluate_due(&pool).await.unwrap();
    assert_eq!(r.events_emitted, 0, "inactive plan should be skipped");
}

// ============================================================================
// 6. Paused assignment skipped
// ============================================================================

#[tokio::test]
#[serial]
async fn test_paused_assignment_skipped() {
    let pool = setup_db().await;
    let t = tid();
    let asset = mk_asset(&pool, &t, "PAUSE-001").await;
    let plan = mk_cal_plan(&pool, &t, "Paused Plan Test", 1).await;
    let asn = assign(&pool, plan, &t, asset).await;

    let yesterday = (Utc::now() - Duration::days(1)).date_naive();
    sqlx::query(
        "UPDATE maintenance_plan_assignments SET next_due_date = $1, state = 'paused' WHERE id = $2",
    ).bind(yesterday).bind(asn).execute(&pool).await.unwrap();

    let r = evaluate_due(&pool).await.unwrap();
    assert_eq!(r.events_emitted, 0, "paused assignment should be skipped");
}
