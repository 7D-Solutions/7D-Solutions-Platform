//! Integration tests for stable NATS subjects and EventEnvelope compliance (bd-16az).
//!
//! Covers:
//! 1. Work order created → outbox event_type matches subjects::WO_CREATED
//! 2. Work order transition → outbox event_type matches subjects::WO_STATUS_CHANGED
//! 3. Meter reading recorded → outbox event_type matches subjects::METER_READING_RECORDED
//! 4. Envelope compliance: every outbox payload has tenant_id, event_id,
//!    occurred_at, source_module, correlation_id (optional but present if set)
//! 5. All subjects in ALL_SUBJECTS start with "maintenance."

use maintenance_rs::domain::assets::{AssetRepo, CreateAssetRequest};
use maintenance_rs::domain::meters::{
    CreateMeterTypeRequest, MeterReadingRepo, MeterTypeRepo, RecordReadingRequest,
};
use maintenance_rs::domain::work_orders::{
    CreateWorkOrderRequest, TransitionRequest, WorkOrderRepo,
};
use maintenance_rs::events::subjects;
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
    format!("evt-test-{}", Uuid::new_v4().simple())
}

async fn create_test_asset(pool: &sqlx::PgPool, tid: &str, tag: &str) -> Uuid {
    AssetRepo::create(
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
    .unwrap()
    .id
}

/// Fetch the latest outbox event for an aggregate.
async fn fetch_outbox_event(
    pool: &sqlx::PgPool,
    aggregate_id: &str,
) -> (String, serde_json::Value) {
    let row: (String, serde_json::Value) = sqlx::query_as(
        r#"SELECT event_type, payload
           FROM events_outbox
           WHERE aggregate_id = $1
           ORDER BY created_at DESC LIMIT 1"#,
    )
    .bind(aggregate_id)
    .fetch_one(pool)
    .await
    .expect("No outbox event found");
    row
}

// ============================================================================
// 1. WO created event uses stable subject
// ============================================================================

#[tokio::test]
#[serial]
async fn test_wo_created_subject_and_envelope() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let asset_id = create_test_asset(&pool, &tid, "EVT-WO1").await;

    let wo = WorkOrderRepo::create(
        &pool,
        &CreateWorkOrderRequest {
            tenant_id: tid.clone(),
            asset_id,
            plan_assignment_id: None,
            title: "Test event subject".into(),
            description: None,
            wo_type: "corrective".into(),
            priority: None,
            assigned_to: None,
            scheduled_date: None,
            checklist: None,
            notes: None,
        },
    )
    .await
    .unwrap();

    let (event_type, payload) = fetch_outbox_event(&pool, &wo.id.to_string()).await;

    // Subject must match the stable constant
    assert_eq!(event_type, subjects::WO_CREATED);

    // Envelope compliance: required fields present and non-empty
    assert!(payload["event_id"].is_string(), "event_id missing");
    assert!(payload["tenant_id"].is_string(), "tenant_id missing");
    assert_eq!(payload["tenant_id"].as_str().unwrap(), tid);
    assert!(payload["occurred_at"].is_string(), "occurred_at missing");
    assert_eq!(payload["source_module"].as_str().unwrap(), "maintenance");
    assert!(payload["event_type"].is_string(), "event_type missing");
    assert_eq!(
        payload["event_type"].as_str().unwrap(),
        subjects::WO_CREATED
    );
}

// ============================================================================
// 2. WO transition event uses stable subject
// ============================================================================

#[tokio::test]
#[serial]
async fn test_wo_transition_subject_and_envelope() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let asset_id = create_test_asset(&pool, &tid, "EVT-WO2").await;

    let wo = WorkOrderRepo::create(
        &pool,
        &CreateWorkOrderRequest {
            tenant_id: tid.clone(),
            asset_id,
            plan_assignment_id: None,
            title: "Transition subject test".into(),
            description: None,
            wo_type: "corrective".into(),
            priority: None,
            assigned_to: None,
            scheduled_date: None,
            checklist: None,
            notes: None,
        },
    )
    .await
    .unwrap();

    // Transition draft → scheduled
    WorkOrderRepo::transition(
        &pool,
        wo.id,
        &TransitionRequest {
            tenant_id: tid.clone(),
            status: "scheduled".into(),
            completed_at: None,
            downtime_minutes: None,
            closed_at: None,
            notes: None,
        },
    )
    .await
    .unwrap();

    let (event_type, payload) = fetch_outbox_event(&pool, &wo.id.to_string()).await;

    assert_eq!(event_type, subjects::WO_STATUS_CHANGED);
    assert_eq!(payload["source_module"].as_str().unwrap(), "maintenance");
    assert_eq!(payload["tenant_id"].as_str().unwrap(), tid);
    assert!(payload["event_id"].is_string());
    assert!(payload["occurred_at"].is_string());
}

// ============================================================================
// 3. Meter reading event uses stable subject
// ============================================================================

#[tokio::test]
#[serial]
async fn test_meter_reading_subject_and_envelope() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let asset_id = create_test_asset(&pool, &tid, "EVT-MTR").await;

    let meter_type = MeterTypeRepo::create(
        &pool,
        &CreateMeterTypeRequest {
            tenant_id: tid.clone(),
            name: format!("Odometer-{}", Uuid::new_v4().simple()),
            unit_label: "km".into(),
            rollover_value: None,
        },
    )
    .await
    .unwrap();

    let reading = MeterReadingRepo::record(
        &pool,
        asset_id,
        &RecordReadingRequest {
            tenant_id: tid.clone(),
            meter_type_id: meter_type.id,
            reading_value: 1000,
            recorded_at: None,
            recorded_by: None,
        },
    )
    .await
    .unwrap();

    let (event_type, payload) = fetch_outbox_event(&pool, &reading.id.to_string()).await;

    assert_eq!(event_type, subjects::METER_READING_RECORDED);
    assert_eq!(payload["source_module"].as_str().unwrap(), "maintenance");
    assert_eq!(payload["tenant_id"].as_str().unwrap(), tid);
    assert!(payload["event_id"].is_string());
    assert!(payload["occurred_at"].is_string());
}

// ============================================================================
// 4. ALL_SUBJECTS exhaustive coverage
// ============================================================================

#[test]
fn all_subjects_are_stable_and_prefixed() {
    for subject in subjects::ALL_SUBJECTS {
        assert!(
            subject.starts_with("maintenance."),
            "Subject '{}' does not start with 'maintenance.'",
            subject
        );
        // Must have at least 3 dot-separated segments
        let segments: Vec<&str> = subject.split('.').collect();
        assert!(
            segments.len() >= 3,
            "Subject '{}' must have >= 3 segments (module.entity.action)",
            subject
        );
    }
    // Exactly 9 stable subjects
    assert_eq!(subjects::ALL_SUBJECTS.len(), 9);
}

// ============================================================================
// 5. Envelope source_version matches Cargo.toml
// ============================================================================

#[tokio::test]
#[serial]
async fn test_envelope_source_version() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let asset_id = create_test_asset(&pool, &tid, "EVT-VER").await;

    let wo = WorkOrderRepo::create(
        &pool,
        &CreateWorkOrderRequest {
            tenant_id: tid.clone(),
            asset_id,
            plan_assignment_id: None,
            title: "Version check".into(),
            description: None,
            wo_type: "corrective".into(),
            priority: None,
            assigned_to: None,
            scheduled_date: None,
            checklist: None,
            notes: None,
        },
    )
    .await
    .unwrap();

    let (_, payload) = fetch_outbox_event(&pool, &wo.id.to_string()).await;

    assert_eq!(
        payload["source_version"].as_str().unwrap(),
        env!("CARGO_PKG_VERSION"),
        "source_version must match Cargo.toml version"
    );
}
