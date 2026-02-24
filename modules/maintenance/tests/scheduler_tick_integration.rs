//! Integration tests for the maintenance scheduler tick (bd-1wuh).
//!
//! Covers:
//! 1. Calendar-due assignment: tick emits maintenance.plan.due, sets due_notified_at
//! 2. Meter-due assignment: tick emits event when reading exceeds threshold
//! 3. Both-schedule: tick fires when calendar condition met
//! 4. Idempotency: second tick does not re-emit for already-notified assignments
//! 5. Inactive plan skipped: assignments under inactive plans are not evaluated
//! 6. Paused assignment skipped: paused state prevents evaluation

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

fn unique_tenant() -> String {
    format!("sched-test-{}", Uuid::new_v4().simple())
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

/// Count outbox events of a given type for a specific aggregate_id.
async fn count_outbox_events(
    pool: &sqlx::PgPool,
    event_type: &str,
    aggregate_id: &str,
) -> i64 {
    let row: (i64,) = sqlx::query_as(
        r#"
        SELECT COUNT(*) FROM events_outbox
        WHERE event_type = $1 AND aggregate_id = $2
        "#,
    )
    .bind(event_type)
    .bind(aggregate_id)
    .fetch_one(pool)
    .await
    .unwrap();
    row.0
}

// ============================================================================
// 1. Calendar-due: next_due_date in the past triggers event
// ============================================================================

#[tokio::test]
#[serial]
async fn test_scheduler_emits_event_for_calendar_due_assignment() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let asset_id = create_test_asset(&pool, &tid, "SCHED-CAL-001").await;

    let plan = PlanRepo::create(
        &pool,
        &CreatePlanRequest {
            tenant_id: tid.clone(),
            name: "Daily Check".into(),
            description: None,
            asset_type_filter: None,
            schedule_type: "calendar".into(),
            calendar_interval_days: Some(1),
            meter_type_id: None,
            meter_interval: None,
            priority: Some("high".into()),
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

    // Backdate next_due_date to yesterday so it's due now
    let yesterday = (Utc::now() - Duration::days(1)).date_naive();
    sqlx::query(
        "UPDATE maintenance_plan_assignments SET next_due_date = $1 WHERE id = $2",
    )
    .bind(yesterday)
    .bind(assignment.id)
    .execute(&pool)
    .await
    .unwrap();

    // Run scheduler tick
    let result = evaluate_due(&pool).await.unwrap();
    assert_eq!(result.evaluated, 1);
    assert_eq!(result.events_emitted, 1);

    // Verify outbox event created
    let count =
        count_outbox_events(&pool, "maintenance.plan.due", &assignment.id.to_string())
            .await;
    assert_eq!(count, 1, "expected exactly 1 plan.due event in outbox");

    // Verify due_notified_at is set
    let updated = AssignmentRepo::find_by_id(&pool, assignment.id, &tid)
        .await
        .unwrap()
        .unwrap();
    assert!(
        updated.due_notified_at.is_some(),
        "due_notified_at should be set after scheduler tick"
    );
}

// ============================================================================
// 2. Meter-due: reading exceeds threshold triggers event
// ============================================================================

#[tokio::test]
#[serial]
async fn test_scheduler_emits_event_for_meter_due_assignment() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let asset_id = create_test_asset(&pool, &tid, "SCHED-MTR-001").await;
    let meter_id = create_test_meter(&pool, &tid, "Odometer").await;

    // Record initial reading
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

    // next_due_meter should be 55000, reading is 50000 — not yet due
    assert_eq!(assignment.next_due_meter, Some(55_000));

    let result = evaluate_due(&pool).await.unwrap();
    assert_eq!(result.events_emitted, 0, "should not be due yet");

    // Record reading that exceeds threshold
    MeterReadingRepo::record(
        &pool,
        asset_id,
        &RecordReadingRequest {
            tenant_id: tid.clone(),
            meter_type_id: meter_id,
            reading_value: 56_000,
            recorded_at: None,
            recorded_by: None,
        },
    )
    .await
    .unwrap();

    // Now it should be due
    let result = evaluate_due(&pool).await.unwrap();
    assert_eq!(result.evaluated, 1);
    assert_eq!(result.events_emitted, 1);

    let count =
        count_outbox_events(&pool, "maintenance.plan.due", &assignment.id.to_string())
            .await;
    assert_eq!(count, 1);
}

// ============================================================================
// 3. Both-schedule: calendar trigger fires
// ============================================================================

#[tokio::test]
#[serial]
async fn test_scheduler_both_schedule_calendar_trigger() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let asset_id = create_test_asset(&pool, &tid, "SCHED-BOTH-001").await;
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
            calendar_interval_days: Some(30),
            meter_type_id: Some(meter_id),
            meter_interval: Some(500),
            priority: Some("critical".into()),
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

    // Backdate calendar to yesterday (meter not yet due)
    let yesterday = (Utc::now() - Duration::days(1)).date_naive();
    sqlx::query(
        "UPDATE maintenance_plan_assignments SET next_due_date = $1 WHERE id = $2",
    )
    .bind(yesterday)
    .bind(assignment.id)
    .execute(&pool)
    .await
    .unwrap();

    let result = evaluate_due(&pool).await.unwrap();
    assert_eq!(result.events_emitted, 1);

    // Verify event payload has trigger info
    let event_payload: serde_json::Value = sqlx::query_scalar(
        r#"
        SELECT payload FROM events_outbox
        WHERE event_type = 'maintenance.plan.due' AND aggregate_id = $1
        ORDER BY created_at DESC LIMIT 1
        "#,
    )
    .bind(assignment.id.to_string())
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(event_payload["plan_priority"], "critical");
    assert_eq!(event_payload["assignment_id"], assignment.id.to_string());
    assert_eq!(event_payload["plan_id"], plan.id.to_string());
    assert_eq!(event_payload["asset_id"], asset_id.to_string());
}

// ============================================================================
// 4. Idempotency: second tick does not re-emit
// ============================================================================

#[tokio::test]
#[serial]
async fn test_scheduler_idempotent_no_double_emission() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let asset_id = create_test_asset(&pool, &tid, "SCHED-IDEM-001").await;

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

    // Backdate to make it due
    let past = (Utc::now() - Duration::days(10)).date_naive();
    sqlx::query(
        "UPDATE maintenance_plan_assignments SET next_due_date = $1 WHERE id = $2",
    )
    .bind(past)
    .bind(assignment.id)
    .execute(&pool)
    .await
    .unwrap();

    // First tick — should emit
    let r1 = evaluate_due(&pool).await.unwrap();
    assert_eq!(r1.events_emitted, 1);

    // Second tick — should NOT emit (due_notified_at is set)
    let r2 = evaluate_due(&pool).await.unwrap();
    assert_eq!(r2.events_emitted, 0);

    // Still only 1 event in outbox
    let count =
        count_outbox_events(&pool, "maintenance.plan.due", &assignment.id.to_string())
            .await;
    assert_eq!(count, 1);
}

// ============================================================================
// 5. Inactive plan skipped
// ============================================================================

#[tokio::test]
#[serial]
async fn test_scheduler_skips_inactive_plan() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let asset_id = create_test_asset(&pool, &tid, "SCHED-INACT-001").await;

    let plan = PlanRepo::create(
        &pool,
        &CreatePlanRequest {
            tenant_id: tid.clone(),
            name: "Deactivated Plan".into(),
            description: None,
            asset_type_filter: None,
            schedule_type: "calendar".into(),
            calendar_interval_days: Some(1),
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

    // Backdate + deactivate plan
    let yesterday = (Utc::now() - Duration::days(1)).date_naive();
    sqlx::query(
        "UPDATE maintenance_plan_assignments SET next_due_date = $1 WHERE id = $2",
    )
    .bind(yesterday)
    .bind(assignment.id)
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query("UPDATE maintenance_plans SET is_active = false WHERE id = $1")
        .bind(plan.id)
        .execute(&pool)
        .await
        .unwrap();

    let result = evaluate_due(&pool).await.unwrap();
    assert_eq!(result.events_emitted, 0, "inactive plan should be skipped");
}

// ============================================================================
// 6. Paused assignment skipped
// ============================================================================

#[tokio::test]
#[serial]
async fn test_scheduler_skips_paused_assignment() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let asset_id = create_test_asset(&pool, &tid, "SCHED-PAUSE-001").await;

    let plan = PlanRepo::create(
        &pool,
        &CreatePlanRequest {
            tenant_id: tid.clone(),
            name: "Paused Plan Test".into(),
            description: None,
            asset_type_filter: None,
            schedule_type: "calendar".into(),
            calendar_interval_days: Some(1),
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

    // Backdate + pause assignment
    let yesterday = (Utc::now() - Duration::days(1)).date_naive();
    sqlx::query(
        "UPDATE maintenance_plan_assignments SET next_due_date = $1, state = 'paused' WHERE id = $2",
    )
    .bind(yesterday)
    .bind(assignment.id)
    .execute(&pool)
    .await
    .unwrap();

    let result = evaluate_due(&pool).await.unwrap();
    assert_eq!(result.events_emitted, 0, "paused assignment should be skipped");
}
