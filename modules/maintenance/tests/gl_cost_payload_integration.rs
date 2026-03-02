//! Integration tests for GL cost payload in completed event (bd-1lxj).
//!
//! Covers:
//! 1. Complete WO with parts + labor → event payload has correct cost totals
//! 2. Complete WO with no parts/labor → event payload has zero totals, default currency
//! 3. Complete WO with fixed_asset_ref on asset → event payload includes it

use maintenance_rs::domain::assets::{AssetRepo, CreateAssetRequest};
use maintenance_rs::domain::work_orders::{
    AddLaborRequest, AddPartRequest, CreateWorkOrderRequest, TransitionRequest, WoLaborRepo,
    WoPartsRepo, WorkOrderRepo,
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
    format!("gl-test-{}", Uuid::new_v4().simple())
}

async fn create_test_asset(
    pool: &sqlx::PgPool,
    tid: &str,
    tag: &str,
    fixed_asset_ref: Option<Uuid>,
) -> Uuid {
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
            fixed_asset_ref,
            metadata: None,
        },
    )
    .await
    .unwrap();
    asset.id
}

/// Create a WO and advance it to in_progress.
async fn create_in_progress_wo(pool: &sqlx::PgPool, tid: &str, asset_id: Uuid) -> Uuid {
    let wo = WorkOrderRepo::create(
        pool,
        &CreateWorkOrderRequest {
            tenant_id: tid.to_string(),
            asset_id,
            plan_assignment_id: None,
            title: "GL Test WO".into(),
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

    // draft → scheduled → in_progress
    for status in &["scheduled", "in_progress"] {
        WorkOrderRepo::transition(
            pool,
            wo.id,
            &TransitionRequest {
                tenant_id: tid.to_string(),
                status: status.to_string(),
                completed_at: None,
                downtime_minutes: None,
                closed_at: None,
                notes: None,
            },
        )
        .await
        .unwrap();
    }

    wo.id
}

/// Fetch the completed event envelope from the outbox for a given WO.
///
/// Returns the full EventEnvelope JSON stored in the outbox. The domain
/// payload lives under the "payload" key of the envelope.
async fn fetch_completed_event_envelope(pool: &sqlx::PgPool, wo_id: Uuid) -> serde_json::Value {
    let row: (serde_json::Value,) = sqlx::query_as(
        r#"SELECT payload FROM events_outbox
           WHERE aggregate_id = $1
             AND event_type = 'maintenance.work_order.completed'
           ORDER BY created_at DESC
           LIMIT 1"#,
    )
    .bind(wo_id.to_string())
    .fetch_one(pool)
    .await
    .expect("completed event not found in outbox");

    row.0
}

// ============================================================================
// 1. Complete WO with parts + labor → correct cost totals
// ============================================================================

#[tokio::test]
#[serial]
async fn test_completed_event_has_cost_totals() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let fa_ref = Uuid::new_v4();
    let asset_id = create_test_asset(&pool, &tid, "GL-001", Some(fa_ref)).await;
    let wo_id = create_in_progress_wo(&pool, &tid, asset_id).await;

    // Add 2x parts: 3 units @ 1000 minor = 3000, 1 unit @ 500 = 500 → total = 3500
    WoPartsRepo::add(
        &pool,
        wo_id,
        &AddPartRequest {
            tenant_id: tid.clone(),
            part_description: "Brake pad".into(),
            part_ref: None,
            quantity: 3,
            unit_cost_minor: 1000,
            currency: Some("USD".into()),
            inventory_issue_ref: None,
        },
    )
    .await
    .unwrap();

    WoPartsRepo::add(
        &pool,
        wo_id,
        &AddPartRequest {
            tenant_id: tid.clone(),
            part_description: "Bolt".into(),
            part_ref: None,
            quantity: 1,
            unit_cost_minor: 500,
            currency: Some("USD".into()),
            inventory_issue_ref: None,
        },
    )
    .await
    .unwrap();

    // Add labor: 2.50 hours @ 8000 minor/hr = 20000
    WoLaborRepo::add(
        &pool,
        wo_id,
        &AddLaborRequest {
            tenant_id: tid.clone(),
            technician_ref: "tech-gl".into(),
            hours_decimal: "2.50".into(),
            rate_minor: 8000,
            currency: Some("USD".into()),
            description: None,
        },
    )
    .await
    .unwrap();

    // Complete the WO
    let now = chrono::Utc::now();
    WorkOrderRepo::transition(
        &pool,
        wo_id,
        &TransitionRequest {
            tenant_id: tid.clone(),
            status: "completed".into(),
            completed_at: Some(now),
            downtime_minutes: Some(60),
            closed_at: None,
            notes: None,
        },
    )
    .await
    .unwrap();

    // Assert event payload (domain data nested in envelope's "payload" field)
    let envelope = fetch_completed_event_envelope(&pool, wo_id).await;
    let inner = &envelope["payload"];

    assert_eq!(inner["total_parts_minor"], 3500);
    assert_eq!(inner["total_labor_minor"], 20000);
    assert_eq!(inner["currency"], "USD");
    assert_eq!(inner["fixed_asset_ref"], fa_ref.to_string());
    assert_eq!(inner["to_status"], "completed");
    assert_eq!(envelope["source_module"], "maintenance");
    assert_eq!(envelope["event_type"], "maintenance.work_order.completed");
}

// ============================================================================
// 2. Complete WO with no parts/labor → zero totals, default currency
// ============================================================================

#[tokio::test]
#[serial]
async fn test_completed_event_zero_cost_when_no_parts_labor() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let asset_id = create_test_asset(&pool, &tid, "GL-002", None).await;
    let wo_id = create_in_progress_wo(&pool, &tid, asset_id).await;

    let now = chrono::Utc::now();
    WorkOrderRepo::transition(
        &pool,
        wo_id,
        &TransitionRequest {
            tenant_id: tid.clone(),
            status: "completed".into(),
            completed_at: Some(now),
            downtime_minutes: Some(0),
            closed_at: None,
            notes: None,
        },
    )
    .await
    .unwrap();

    let envelope = fetch_completed_event_envelope(&pool, wo_id).await;
    let inner = &envelope["payload"];

    assert_eq!(inner["total_parts_minor"], 0);
    assert_eq!(inner["total_labor_minor"], 0);
    assert_eq!(inner["currency"], "USD");
    assert!(inner["fixed_asset_ref"].is_null());
}

// ============================================================================
// 3. Non-completed transitions do NOT include cost fields
// ============================================================================

#[tokio::test]
#[serial]
async fn test_non_completed_transition_has_no_cost_fields() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let asset_id = create_test_asset(&pool, &tid, "GL-003", None).await;

    let wo = WorkOrderRepo::create(
        &pool,
        &CreateWorkOrderRequest {
            tenant_id: tid.clone(),
            asset_id,
            plan_assignment_id: None,
            title: "GL No-Cost WO".into(),
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

    // draft → scheduled
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

    // Check the status_changed event payload for scheduled transition
    let row: (serde_json::Value,) = sqlx::query_as(
        r#"SELECT payload FROM events_outbox
           WHERE aggregate_id = $1
             AND event_type = 'maintenance.work_order.status_changed'
           ORDER BY created_at DESC
           LIMIT 1"#,
    )
    .bind(wo.id.to_string())
    .fetch_one(&pool)
    .await
    .unwrap();

    let envelope = row.0;
    let inner = &envelope["payload"];
    assert!(inner.get("total_parts_minor").is_none());
    assert!(inner.get("total_labor_minor").is_none());
    assert!(inner.get("currency").is_none());
    assert!(inner.get("fixed_asset_ref").is_none());
}
