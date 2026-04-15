//! Integration tests for calibration status + downtime extensions (bd-20rpu).
//!
//! Covers:
//! 1. Calibration event recording + status derivation
//! 2. Out-of-service override on calibration status
//! 3. Calibration status transitions are deterministic
//! 4. Downtime with workcenter_id (no asset_id)
//! 5. Downtime date range filtering
//! 6. Tenant isolation for calibration events
//! 7. Outbox events for new event types

use chrono::{Duration, Utc};
use maintenance_rs::domain::assets::{AssetRepo, CreateAssetRequest, UpdateAssetRequest};
use maintenance_rs::domain::calibration_events::{
    CalibrationEventRepo, CalibrationStatus, RecordCalibrationRequest,
};
use maintenance_rs::domain::downtime::{CreateDowntimeRequest, DowntimeRepo, ListDowntimeQuery};
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
    format!("calst-{}", Uuid::new_v4().simple())
}

async fn mk_asset(pool: &sqlx::PgPool, tenant_id: &str, tag: &str) -> Uuid {
    AssetRepo::create(
        pool,
        &CreateAssetRequest {
            tenant_id: tenant_id.into(),
            asset_tag: tag.into(),
            name: format!("Asset {tag}"),
            description: None,
            asset_type: "equipment".into(),
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

async fn cleanup_tenant(pool: &sqlx::PgPool, tenant_id: &str) {
    let tables = [
        "calibration_events",
        "calibration_records",
        "downtime_events",
        "events_outbox",
        "work_order_labor",
        "work_order_parts",
        "work_orders",
        "meter_readings",
        "maintenance_plan_assignments",
        "maintenance_plans",
        "meter_types",
        "maintainable_assets",
        "wo_counters",
        "maintenance_tenant_config",
    ];
    for table in &tables {
        let sql = format!("DELETE FROM {} WHERE tenant_id = $1", table);
        sqlx::query(&sql).bind(tenant_id).execute(pool).await.ok();
    }
}

// ============================================================================
// 1. Calibration event recording + status derivation
// ============================================================================

#[tokio::test]
#[serial]
async fn test_calibration_event_record_and_status() {
    let pool = setup_db().await;
    let t = tid();
    let asset_id = mk_asset(&pool, &t, "CALST-001").await;

    // Record a calibration event — due 30 days from now
    let performed = Utc::now();
    let due = performed + Duration::days(30);

    let event = CalibrationEventRepo::record(
        &pool,
        asset_id,
        &RecordCalibrationRequest {
            tenant_id: t.clone(),
            performed_at: performed,
            due_at: due,
            result: "pass".into(),
            doc_revision_id: None,
            idempotency_key: Some(format!("calst-1-{}", Uuid::new_v4())),
        },
    )
    .await
    .unwrap();

    assert_eq!(event.tenant_id, t);
    assert_eq!(event.asset_id, asset_id);
    assert_eq!(event.result, "pass");

    // Check status — should be in_cal (due is in the future)
    let status = CalibrationEventRepo::get_status(&pool, asset_id, &t)
        .await
        .unwrap();
    assert_eq!(status.status, CalibrationStatus::InCal);
    assert!(status.last_calibrated_at.is_some());
    assert!(status.next_due_at.is_some());

    cleanup_tenant(&pool, &t).await;
}

// ============================================================================
// 2. Out-of-service override on calibration status
// ============================================================================

#[tokio::test]
#[serial]
async fn test_out_of_service_overrides_calibration_status() {
    let pool = setup_db().await;
    let t = tid();
    let asset_id = mk_asset(&pool, &t, "CALST-OOS-001").await;

    // Record a valid calibration
    let performed = Utc::now();
    let due = performed + Duration::days(30);
    CalibrationEventRepo::record(
        &pool,
        asset_id,
        &RecordCalibrationRequest {
            tenant_id: t.clone(),
            performed_at: performed,
            due_at: due,
            result: "pass".into(),
            doc_revision_id: None,
            idempotency_key: None,
        },
    )
    .await
    .unwrap();

    // Status should be in_cal
    let status = CalibrationEventRepo::get_status(&pool, asset_id, &t)
        .await
        .unwrap();
    assert_eq!(status.status, CalibrationStatus::InCal);

    // Set asset out of service
    AssetRepo::update(
        &pool,
        asset_id,
        &t,
        &UpdateAssetRequest {
            name: None,
            description: None,
            asset_type: None,
            location: None,
            department: None,
            responsible_person: None,
            serial_number: None,
            fixed_asset_ref: None,
            status: None,
            metadata: None,
            maintenance_schedule: None,
            out_of_service: Some(true),
            out_of_service_reason: Some("Safety recall".into()),
        },
    )
    .await
    .unwrap();

    // Status should now be out_of_service regardless of calibration
    let status = CalibrationEventRepo::get_status(&pool, asset_id, &t)
        .await
        .unwrap();
    assert_eq!(status.status, CalibrationStatus::OutOfService);
    // Still shows last calibration info
    assert!(status.last_calibrated_at.is_some());

    // Restore from out of service
    AssetRepo::update(
        &pool,
        asset_id,
        &t,
        &UpdateAssetRequest {
            name: None,
            description: None,
            asset_type: None,
            location: None,
            department: None,
            responsible_person: None,
            serial_number: None,
            fixed_asset_ref: None,
            status: None,
            metadata: None,
            maintenance_schedule: None,
            out_of_service: Some(false),
            out_of_service_reason: None,
        },
    )
    .await
    .unwrap();

    // Status should revert to in_cal
    let status = CalibrationEventRepo::get_status(&pool, asset_id, &t)
        .await
        .unwrap();
    assert_eq!(status.status, CalibrationStatus::InCal);

    cleanup_tenant(&pool, &t).await;
}

// ============================================================================
// 3. Calibration status transitions are deterministic
// ============================================================================

#[tokio::test]
#[serial]
async fn test_calibration_status_deterministic() {
    let pool = setup_db().await;
    let t = tid();
    let asset_id = mk_asset(&pool, &t, "CALST-DET-001").await;

    // No calibration events → overdue
    let status = CalibrationEventRepo::get_status(&pool, asset_id, &t)
        .await
        .unwrap();
    assert_eq!(status.status, CalibrationStatus::Overdue);
    assert!(status.last_calibrated_at.is_none());
    assert!(status.next_due_at.is_none());

    // Record an already-expired calibration (due in the past)
    CalibrationEventRepo::record(
        &pool,
        asset_id,
        &RecordCalibrationRequest {
            tenant_id: t.clone(),
            performed_at: Utc::now() - Duration::days(60),
            due_at: Utc::now() - Duration::days(1),
            result: "pass".into(),
            doc_revision_id: None,
            idempotency_key: None,
        },
    )
    .await
    .unwrap();

    // Status should be overdue (due_at is in the past)
    let status = CalibrationEventRepo::get_status(&pool, asset_id, &t)
        .await
        .unwrap();
    assert_eq!(status.status, CalibrationStatus::Overdue);

    // Record a new valid calibration
    CalibrationEventRepo::record(
        &pool,
        asset_id,
        &RecordCalibrationRequest {
            tenant_id: t.clone(),
            performed_at: Utc::now(),
            due_at: Utc::now() + Duration::days(90),
            result: "pass".into(),
            doc_revision_id: None,
            idempotency_key: None,
        },
    )
    .await
    .unwrap();

    // Status should be in_cal
    let status = CalibrationEventRepo::get_status(&pool, asset_id, &t)
        .await
        .unwrap();
    assert_eq!(status.status, CalibrationStatus::InCal);

    cleanup_tenant(&pool, &t).await;
}

// ============================================================================
// 4. Downtime with workcenter_id (no asset_id)
// ============================================================================

#[tokio::test]
#[serial]
async fn test_downtime_workcenter_only() {
    let pool = setup_db().await;
    let t = tid();
    let wc_id = Uuid::new_v4();

    let dt = DowntimeRepo::create(
        &pool,
        &CreateDowntimeRequest {
            tenant_id: t.clone(),
            asset_id: None,
            start_time: Utc::now() - Duration::hours(2),
            end_time: Some(Utc::now() - Duration::hours(1)),
            reason: "Line shutdown".into(),
            impact_classification: "major".into(),
            idempotency_key: None,
            notes: None,
            workcenter_id: Some(wc_id),
            reason_code: Some("MAINT_SCHED".into()),
            wo_ref: Some("WO-2026-001".into()),
        },
    )
    .await
    .unwrap();

    assert_eq!(dt.asset_id, None);
    assert_eq!(dt.workcenter_id, Some(wc_id));
    assert_eq!(dt.reason_code.as_deref(), Some("MAINT_SCHED"));
    assert_eq!(dt.wo_ref.as_deref(), Some("WO-2026-001"));

    // Read back
    let found = DowntimeRepo::find_by_id(&pool, dt.id, &t)
        .await
        .unwrap()
        .expect("should find workcenter downtime");
    assert_eq!(found.workcenter_id, Some(wc_id));

    cleanup_tenant(&pool, &t).await;
}

// ============================================================================
// 5. Downtime date range filtering
// ============================================================================

#[tokio::test]
#[serial]
async fn test_downtime_date_range_filter() {
    let pool = setup_db().await;
    let t = tid();
    let asset_id = mk_asset(&pool, &t, "CALST-DT-RANGE").await;

    let now = Utc::now();

    // Create 3 downtime events at different times
    for hours_ago in [24, 12, 2] {
        DowntimeRepo::create(
            &pool,
            &CreateDowntimeRequest {
                tenant_id: t.clone(),
                asset_id: Some(asset_id),
                start_time: now - Duration::hours(hours_ago),
                end_time: Some(now - Duration::hours(hours_ago - 1)),
                reason: format!("Downtime {}h ago", hours_ago),
                impact_classification: "minor".into(),
                idempotency_key: None,
                notes: None,
                workcenter_id: None,
                reason_code: None,
                wo_ref: None,
            },
        )
        .await
        .unwrap();
    }

    // Query all → 3 events
    let all = DowntimeRepo::list(
        &pool,
        &ListDowntimeQuery {
            tenant_id: t.clone(),
            asset_id: Some(asset_id),
            from: None,
            to: None,
            limit: None,
            offset: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(all.len(), 3);

    // Query last 6 hours → only 1 event (2h ago)
    let recent = DowntimeRepo::list(
        &pool,
        &ListDowntimeQuery {
            tenant_id: t.clone(),
            asset_id: Some(asset_id),
            from: Some(now - Duration::hours(6)),
            to: None,
            limit: None,
            offset: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(recent.len(), 1);

    // Query between 18h ago and 6h ago → 1 event (12h ago)
    let mid = DowntimeRepo::list(
        &pool,
        &ListDowntimeQuery {
            tenant_id: t.clone(),
            asset_id: Some(asset_id),
            from: Some(now - Duration::hours(18)),
            to: Some(now - Duration::hours(6)),
            limit: None,
            offset: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(mid.len(), 1);

    cleanup_tenant(&pool, &t).await;
}

// ============================================================================
// 6. Tenant isolation for calibration events
// ============================================================================

#[tokio::test]
#[serial]
async fn test_calibration_event_tenant_isolation() {
    let pool = setup_db().await;
    let t_a = tid();
    let t_b = tid();
    let asset_a = mk_asset(&pool, &t_a, "CALST-ISO-A").await;
    let asset_b = mk_asset(&pool, &t_b, "CALST-ISO-B").await;

    // Record calibration for tenant A
    CalibrationEventRepo::record(
        &pool,
        asset_a,
        &RecordCalibrationRequest {
            tenant_id: t_a.clone(),
            performed_at: Utc::now(),
            due_at: Utc::now() + Duration::days(30),
            result: "pass".into(),
            doc_revision_id: None,
            idempotency_key: None,
        },
    )
    .await
    .unwrap();

    // Tenant B cannot see tenant A's calibration status
    let status_b = CalibrationEventRepo::get_status(&pool, asset_a, &t_b).await;
    assert!(
        status_b.is_err(),
        "Tenant B must not see Tenant A's asset calibration status"
    );

    // Tenant B's own asset has no calibration → overdue
    let status_own = CalibrationEventRepo::get_status(&pool, asset_b, &t_b)
        .await
        .unwrap();
    assert_eq!(status_own.status, CalibrationStatus::Overdue);

    cleanup_tenant(&pool, &t_a).await;
    cleanup_tenant(&pool, &t_b).await;
}

// ============================================================================
// 7. Outbox events for new event types
// ============================================================================

#[tokio::test]
#[serial]
async fn test_calibration_event_outbox() {
    let pool = setup_db().await;
    let t = tid();
    let asset_id = mk_asset(&pool, &t, "CALST-EVT-001").await;

    let event = CalibrationEventRepo::record(
        &pool,
        asset_id,
        &RecordCalibrationRequest {
            tenant_id: t.clone(),
            performed_at: Utc::now(),
            due_at: Utc::now() + Duration::days(30),
            result: "pass".into(),
            doc_revision_id: None,
            idempotency_key: None,
        },
    )
    .await
    .unwrap();

    // Verify calibration_event_recorded event in outbox
    let recorded_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox WHERE event_type = 'maintenance.calibration.event_recorded' AND aggregate_id = $1",
    )
    .bind(event.id.to_string())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(recorded_count, 1, "should have calibration.event_recorded");

    // Verify calibration_status_changed event in outbox (keyed by asset_id)
    let status_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox WHERE event_type = 'maintenance.calibration.status_changed' AND aggregate_id = $1",
    )
    .bind(asset_id.to_string())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        status_count >= 1,
        "should have calibration.status_changed event"
    );

    // Verify envelope structure
    let payload: serde_json::Value = sqlx::query_scalar(
        "SELECT payload FROM events_outbox WHERE event_type = 'maintenance.calibration.event_recorded' AND aggregate_id = $1 LIMIT 1",
    )
    .bind(event.id.to_string())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(payload["source_module"], "maintenance");
    assert_eq!(
        payload["event_type"],
        "maintenance.calibration.event_recorded"
    );

    cleanup_tenant(&pool, &t).await;
}

// ============================================================================
// 8. Out-of-service change emits event
// ============================================================================

#[tokio::test]
#[serial]
async fn test_out_of_service_change_emits_event() {
    let pool = setup_db().await;
    let t = tid();
    let asset_id = mk_asset(&pool, &t, "CALST-OOS-EVT").await;

    // Set out of service
    AssetRepo::update(
        &pool,
        asset_id,
        &t,
        &UpdateAssetRequest {
            name: None,
            description: None,
            asset_type: None,
            location: None,
            department: None,
            responsible_person: None,
            serial_number: None,
            fixed_asset_ref: None,
            status: None,
            metadata: None,
            maintenance_schedule: None,
            out_of_service: Some(true),
            out_of_service_reason: Some("Faulty sensor".into()),
        },
    )
    .await
    .unwrap();

    // Verify out_of_service_changed event in outbox
    let oos_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox WHERE event_type = 'maintenance.asset.out_of_service_changed' AND aggregate_id = $1",
    )
    .bind(asset_id.to_string())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(oos_count, 1, "should have out_of_service_changed event");

    // Verify envelope payload
    let payload: serde_json::Value = sqlx::query_scalar(
        "SELECT payload FROM events_outbox WHERE event_type = 'maintenance.asset.out_of_service_changed' AND aggregate_id = $1 LIMIT 1",
    )
    .bind(asset_id.to_string())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(payload["payload"]["out_of_service"], true);
    assert_eq!(payload["payload"]["out_of_service_reason"], "Faulty sensor");

    cleanup_tenant(&pool, &t).await;
}

// ============================================================================
// 9. Validation: invalid calibration result rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn test_calibration_event_validates_result() {
    let pool = setup_db().await;
    let t = tid();
    let asset_id = mk_asset(&pool, &t, "CALST-VAL-001").await;

    let err = CalibrationEventRepo::record(
        &pool,
        asset_id,
        &RecordCalibrationRequest {
            tenant_id: t.clone(),
            performed_at: Utc::now(),
            due_at: Utc::now() + Duration::days(30),
            result: "invalid_result".into(),
            doc_revision_id: None,
            idempotency_key: None,
        },
    )
    .await;
    assert!(err.is_err(), "invalid result should be rejected");

    // due_at before performed_at should also fail
    let err2 = CalibrationEventRepo::record(
        &pool,
        asset_id,
        &RecordCalibrationRequest {
            tenant_id: t.clone(),
            performed_at: Utc::now(),
            due_at: Utc::now() - Duration::days(1),
            result: "pass".into(),
            doc_revision_id: None,
            idempotency_key: None,
        },
    )
    .await;
    assert!(
        err2.is_err(),
        "due_at before performed_at should be rejected"
    );

    cleanup_tenant(&pool, &t).await;
}

// ============================================================================
// 10. Downtime requires at least asset_id or workcenter_id
// ============================================================================

#[tokio::test]
#[serial]
async fn test_downtime_requires_asset_or_workcenter() {
    let pool = setup_db().await;
    let t = tid();

    let err = DowntimeRepo::create(
        &pool,
        &CreateDowntimeRequest {
            tenant_id: t.clone(),
            asset_id: None,
            start_time: Utc::now() - Duration::hours(1),
            end_time: Some(Utc::now()),
            reason: "No target".into(),
            impact_classification: "minor".into(),
            idempotency_key: None,
            notes: None,
            workcenter_id: None,
            reason_code: None,
            wo_ref: None,
        },
    )
    .await;
    assert!(
        err.is_err(),
        "downtime without asset_id or workcenter_id should be rejected"
    );

    cleanup_tenant(&pool, &t).await;
}
