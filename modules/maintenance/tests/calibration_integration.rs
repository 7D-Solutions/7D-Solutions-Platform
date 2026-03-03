//! Integration tests for calibration support hooks (bd-32j97).
//!
//! Covers all 6 required test categories:
//! 1. Calibration record E2E — create and verify persistence
//! 2. Calibration completion — mark completed with certificate ref
//! 3. Overdue detection — find calibrations past due date
//! 4. Tenant isolation — no cross-tenant data leakage
//! 5. Idempotency — duplicate idempotency_key returns existing record
//! 6. Outbox events — verify correct event types after create and complete

use chrono::{Duration, Utc};
use maintenance_rs::domain::assets::{AssetRepo, CreateAssetRequest};
use maintenance_rs::domain::calibration::{
    CalibrationRepo, CompleteCalibrationRequest, CreateCalibrationRequest,
};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

// ============================================================================
// Test DB helpers
// ============================================================================

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
    format!("cal-test-{}", Uuid::new_v4().simple())
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
        "calibration_records",
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
// 1. Calibration record E2E — create and verify persistence
// ============================================================================

#[tokio::test]
#[serial]
async fn test_calibration_record_e2e() {
    let pool = setup_db().await;
    let t = tid();
    let asset_id = mk_asset(&pool, &t, "CAL-E2E-001").await;
    let due = (Utc::now() + Duration::days(30)).date_naive();

    let record = CalibrationRepo::create(
        &pool,
        &CreateCalibrationRequest {
            tenant_id: t.clone(),
            asset_id,
            calibration_type: "torque_wrench".into(),
            due_date: due,
            idempotency_key: format!("e2e-{}", Uuid::new_v4()),
        },
    )
    .await
    .unwrap();

    assert_eq!(record.tenant_id, t);
    assert_eq!(record.asset_id, asset_id);
    assert_eq!(record.calibration_type, "torque_wrench");
    assert_eq!(record.due_date, due);
    assert_eq!(record.status, "scheduled");
    assert!(record.completed_date.is_none());
    assert!(record.certificate_ref.is_none());

    // Verify persistence via find_by_id
    let fetched = CalibrationRepo::find_by_id(&pool, record.id, &t)
        .await
        .unwrap()
        .expect("should find the record");
    assert_eq!(fetched.id, record.id);
    assert_eq!(fetched.calibration_type, "torque_wrench");

    cleanup_tenant(&pool, &t).await;
}

// ============================================================================
// 2. Calibration completion — mark completed with certificate ref
// ============================================================================

#[tokio::test]
#[serial]
async fn test_calibration_completion() {
    let pool = setup_db().await;
    let t = tid();
    let asset_id = mk_asset(&pool, &t, "CAL-COMP-001").await;
    let due = (Utc::now() + Duration::days(15)).date_naive();

    let record = CalibrationRepo::create(
        &pool,
        &CreateCalibrationRequest {
            tenant_id: t.clone(),
            asset_id,
            calibration_type: "pressure_gauge".into(),
            due_date: due,
            idempotency_key: format!("comp-{}", Uuid::new_v4()),
        },
    )
    .await
    .unwrap();

    assert_eq!(record.status, "scheduled");

    // Complete the calibration
    let completed = CalibrationRepo::complete(
        &pool,
        record.id,
        &CompleteCalibrationRequest {
            tenant_id: t.clone(),
            certificate_ref: "CERT-2026-001".into(),
            completed_date: None, // uses Utc::now()
        },
    )
    .await
    .unwrap();

    assert_eq!(completed.status, "completed");
    assert_eq!(completed.certificate_ref.as_deref(), Some("CERT-2026-001"));
    assert!(completed.completed_date.is_some());

    // Verify immutability — completing again should fail
    let err = CalibrationRepo::complete(
        &pool,
        record.id,
        &CompleteCalibrationRequest {
            tenant_id: t.clone(),
            certificate_ref: "CERT-2026-002".into(),
            completed_date: None,
        },
    )
    .await;
    assert!(err.is_err(), "Completing an already-completed calibration must fail");

    cleanup_tenant(&pool, &t).await;
}

// ============================================================================
// 3. Overdue detection — find calibrations past due date
// ============================================================================

#[tokio::test]
#[serial]
async fn test_overdue_detection() {
    let pool = setup_db().await;
    let t = tid();
    let asset_id = mk_asset(&pool, &t, "CAL-OD-001").await;

    // Create an overdue calibration (due 5 days ago)
    let overdue_due = (Utc::now() - Duration::days(5)).date_naive();
    let overdue_rec = CalibrationRepo::create(
        &pool,
        &CreateCalibrationRequest {
            tenant_id: t.clone(),
            asset_id,
            calibration_type: "micrometer".into(),
            due_date: overdue_due,
            idempotency_key: format!("od-1-{}", Uuid::new_v4()),
        },
    )
    .await
    .unwrap();

    // Create a future calibration (not overdue)
    let future_due = (Utc::now() + Duration::days(30)).date_naive();
    CalibrationRepo::create(
        &pool,
        &CreateCalibrationRequest {
            tenant_id: t.clone(),
            asset_id,
            calibration_type: "thermometer".into(),
            due_date: future_due,
            idempotency_key: format!("od-2-{}", Uuid::new_v4()),
        },
    )
    .await
    .unwrap();

    // Create a completed calibration with past due date (should NOT appear)
    let completed_due = (Utc::now() - Duration::days(10)).date_naive();
    let completed_rec = CalibrationRepo::create(
        &pool,
        &CreateCalibrationRequest {
            tenant_id: t.clone(),
            asset_id,
            calibration_type: "scale".into(),
            due_date: completed_due,
            idempotency_key: format!("od-3-{}", Uuid::new_v4()),
        },
    )
    .await
    .unwrap();
    CalibrationRepo::complete(
        &pool,
        completed_rec.id,
        &CompleteCalibrationRequest {
            tenant_id: t.clone(),
            certificate_ref: "CERT-DONE".into(),
            completed_date: None,
        },
    )
    .await
    .unwrap();

    // Query overdue
    let overdue = CalibrationRepo::find_overdue(&pool, &t).await.unwrap();

    // Should find exactly the one overdue scheduled record
    assert_eq!(overdue.len(), 1, "should find exactly 1 overdue calibration");
    assert_eq!(overdue[0].id, overdue_rec.id);
    assert_eq!(overdue[0].calibration_type, "micrometer");
    assert!(overdue[0].days_overdue >= 5, "should be at least 5 days overdue");

    cleanup_tenant(&pool, &t).await;
}

// ============================================================================
// 4. Tenant isolation — no cross-tenant data leakage
// ============================================================================

#[tokio::test]
#[serial]
async fn test_tenant_isolation() {
    let pool = setup_db().await;
    let tenant_a = tid();
    let tenant_b = tid();
    let asset_a = mk_asset(&pool, &tenant_a, "CAL-ISO-A").await;
    let asset_b = mk_asset(&pool, &tenant_b, "CAL-ISO-B").await;
    let due = (Utc::now() + Duration::days(20)).date_naive();

    // Create calibration for tenant A
    let rec_a = CalibrationRepo::create(
        &pool,
        &CreateCalibrationRequest {
            tenant_id: tenant_a.clone(),
            asset_id: asset_a,
            calibration_type: "torque_wrench".into(),
            due_date: due,
            idempotency_key: format!("iso-a-{}", Uuid::new_v4()),
        },
    )
    .await
    .unwrap();

    // Create calibration for tenant B
    CalibrationRepo::create(
        &pool,
        &CreateCalibrationRequest {
            tenant_id: tenant_b.clone(),
            asset_id: asset_b,
            calibration_type: "pressure_gauge".into(),
            due_date: due,
            idempotency_key: format!("iso-b-{}", Uuid::new_v4()),
        },
    )
    .await
    .unwrap();

    // Tenant B cannot see tenant A's calibration
    let cross = CalibrationRepo::find_by_id(&pool, rec_a.id, &tenant_b)
        .await
        .unwrap();
    assert!(
        cross.is_none(),
        "Tenant B must NOT see Tenant A's calibration record"
    );

    // Overdue queries are also tenant-scoped — create overdue for A only
    let overdue_due = (Utc::now() - Duration::days(3)).date_naive();
    CalibrationRepo::create(
        &pool,
        &CreateCalibrationRequest {
            tenant_id: tenant_a.clone(),
            asset_id: asset_a,
            calibration_type: "overdue_test".into(),
            due_date: overdue_due,
            idempotency_key: format!("iso-od-{}", Uuid::new_v4()),
        },
    )
    .await
    .unwrap();

    let b_overdue = CalibrationRepo::find_overdue(&pool, &tenant_b).await.unwrap();
    for item in &b_overdue {
        assert_ne!(
            item.tenant_id, tenant_a,
            "Tenant B's overdue query must not include tenant A's records"
        );
    }

    // Direct SQL cross-check
    let cross_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM calibration_records WHERE tenant_id = $1 AND asset_id = $2",
    )
    .bind(&tenant_b)
    .bind(asset_a)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        cross_count, 0,
        "Tenant B must have zero records for tenant A's asset"
    );

    cleanup_tenant(&pool, &tenant_a).await;
    cleanup_tenant(&pool, &tenant_b).await;
}

// ============================================================================
// 5. Idempotency — same idempotency_key returns existing record
// ============================================================================

#[tokio::test]
#[serial]
async fn test_idempotency() {
    let pool = setup_db().await;
    let t = tid();
    let asset_id = mk_asset(&pool, &t, "CAL-IDEMP-001").await;
    let due = (Utc::now() + Duration::days(30)).date_naive();
    let idemp_key = format!("idemp-{}", Uuid::new_v4());

    // First creation
    let first = CalibrationRepo::create(
        &pool,
        &CreateCalibrationRequest {
            tenant_id: t.clone(),
            asset_id,
            calibration_type: "torque_wrench".into(),
            due_date: due,
            idempotency_key: idemp_key.clone(),
        },
    )
    .await
    .unwrap();

    // Second creation with same key — should return same record
    let second = CalibrationRepo::create(
        &pool,
        &CreateCalibrationRequest {
            tenant_id: t.clone(),
            asset_id,
            calibration_type: "torque_wrench".into(),
            due_date: due,
            idempotency_key: idemp_key.clone(),
        },
    )
    .await
    .unwrap();

    assert_eq!(first.id, second.id, "idempotent create must return same record");

    // Verify only one row exists
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM calibration_records WHERE tenant_id = $1 AND idempotency_key = $2",
    )
    .bind(&t)
    .bind(&idemp_key)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 1, "must have exactly 1 record for the idempotency key");

    cleanup_tenant(&pool, &t).await;
}

// ============================================================================
// 6. Outbox events — verify correct event types after create and complete
// ============================================================================

#[tokio::test]
#[serial]
async fn test_outbox_events() {
    let pool = setup_db().await;
    let t = tid();
    let asset_id = mk_asset(&pool, &t, "CAL-EVT-001").await;
    let due = (Utc::now() + Duration::days(30)).date_naive();

    let record = CalibrationRepo::create(
        &pool,
        &CreateCalibrationRequest {
            tenant_id: t.clone(),
            asset_id,
            calibration_type: "dial_indicator".into(),
            due_date: due,
            idempotency_key: format!("evt-{}", Uuid::new_v4()),
        },
    )
    .await
    .unwrap();

    // Verify creation event in outbox
    let created_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox WHERE event_type = 'maintenance.calibration.created' AND aggregate_id = $1",
    )
    .bind(record.id.to_string())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(created_count, 1, "should have 1 calibration.created event");

    // Verify envelope structure
    let payload: serde_json::Value = sqlx::query_scalar(
        "SELECT payload FROM events_outbox WHERE event_type = 'maintenance.calibration.created' AND aggregate_id = $1 LIMIT 1",
    )
    .bind(record.id.to_string())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(payload["source_module"], "maintenance");
    assert_eq!(payload["event_type"], "maintenance.calibration.created");
    assert_eq!(
        payload["payload"]["calibration_id"],
        record.id.to_string()
    );

    // Complete the calibration
    CalibrationRepo::complete(
        &pool,
        record.id,
        &CompleteCalibrationRequest {
            tenant_id: t.clone(),
            certificate_ref: "CERT-EVT-001".into(),
            completed_date: None,
        },
    )
    .await
    .unwrap();

    // Verify completion event in outbox
    let completed_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox WHERE event_type = 'maintenance.calibration.completed' AND aggregate_id = $1",
    )
    .bind(record.id.to_string())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        completed_count, 1,
        "should have 1 calibration.completed event"
    );

    // Verify completion envelope
    let comp_payload: serde_json::Value = sqlx::query_scalar(
        "SELECT payload FROM events_outbox WHERE event_type = 'maintenance.calibration.completed' AND aggregate_id = $1 LIMIT 1",
    )
    .bind(record.id.to_string())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(comp_payload["source_module"], "maintenance");
    assert_eq!(
        comp_payload["event_type"],
        "maintenance.calibration.completed"
    );
    assert_eq!(
        comp_payload["payload"]["certificate_ref"],
        "CERT-EVT-001"
    );

    cleanup_tenant(&pool, &t).await;
}
