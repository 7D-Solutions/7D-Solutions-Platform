//! Integration tests for work order state transitions (bd-3nvm).
//!
//! Covers:
//! 1. Full transition chain: draft → scheduled → in_progress → completed → closed
//! 2. Invalid transition rejected (draft → in_progress)
//! 3. Completion guard: requires completed_at + downtime_minutes
//! 4. Close guard: requires closed_at
//! 5. Cancel from draft
//! 6. Cannot transition from terminal state

use maintenance_rs::domain::assets::{AssetRepo, CreateAssetRequest};
use maintenance_rs::domain::work_orders::{
    CreateWorkOrderRequest, TransitionRequest, WoError, WorkOrderRepo,
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
    format!("wo-tr-{}", Uuid::new_v4().simple())
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
            maintenance_schedule: None,
            idempotency_key: None,
        },
    )
    .await
    .unwrap();
    asset.id
}

fn base_create_req(tid: &str, asset_id: Uuid) -> CreateWorkOrderRequest {
    CreateWorkOrderRequest {
        tenant_id: tid.to_string(),
        asset_id,
        plan_assignment_id: None,
        title: "Test WO".into(),
        description: None,
        wo_type: "corrective".into(),
        priority: None,
        assigned_to: None,
        scheduled_date: None,
        checklist: None,
        notes: None,
    }
}

/// Helper: transition a WO to a given status (no guard fields).
async fn transition_simple(pool: &sqlx::PgPool, wo_id: Uuid, tid: &str, status: &str) {
    WorkOrderRepo::transition(
        pool,
        wo_id,
        &TransitionRequest {
            tenant_id: tid.to_string(),
            status: status.into(),
            completed_at: None,
            downtime_minutes: None,
            closed_at: None,
            notes: None,
        },
    )
    .await
    .unwrap();
}

// ============================================================================
// 1. Full transition chain
// ============================================================================

#[tokio::test]
#[serial]
async fn test_full_transition_chain() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let asset_id = create_test_asset(&pool, &tid, "CHAIN-001").await;

    let wo = WorkOrderRepo::create(&pool, &base_create_req(&tid, asset_id))
        .await
        .unwrap();

    // draft → scheduled
    let wo = WorkOrderRepo::transition(
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
    assert_eq!(wo.status.as_str(), "scheduled");

    // scheduled → in_progress
    let wo = WorkOrderRepo::transition(
        &pool,
        wo.id,
        &TransitionRequest {
            tenant_id: tid.clone(),
            status: "in_progress".into(),
            completed_at: None,
            downtime_minutes: None,
            closed_at: None,
            notes: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(wo.status.as_str(), "in_progress");
    assert!(wo.started_at.is_some());

    // in_progress → completed
    let now = chrono::Utc::now();
    let wo = WorkOrderRepo::transition(
        &pool,
        wo.id,
        &TransitionRequest {
            tenant_id: tid.clone(),
            status: "completed".into(),
            completed_at: Some(now),
            downtime_minutes: Some(45),
            closed_at: None,
            notes: Some("Work done".into()),
        },
    )
    .await
    .unwrap();
    assert_eq!(wo.status.as_str(), "completed");
    assert!(wo.completed_at.is_some());
    assert_eq!(wo.downtime_minutes, Some(45));

    // completed → closed
    let wo = WorkOrderRepo::transition(
        &pool,
        wo.id,
        &TransitionRequest {
            tenant_id: tid.clone(),
            status: "closed".into(),
            completed_at: None,
            downtime_minutes: None,
            closed_at: Some(now),
            notes: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(wo.status.as_str(), "closed");
    assert!(wo.closed_at.is_some());

    // Verify outbox: created + 4 transitions = 5 events
    let count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM events_outbox WHERE aggregate_id = $1")
            .bind(wo.id.to_string())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count.0, 5);
}

// ============================================================================
// 2. Invalid transition rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn test_invalid_transition_rejected() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let asset_id = create_test_asset(&pool, &tid, "BAD-001").await;

    let wo = WorkOrderRepo::create(&pool, &base_create_req(&tid, asset_id))
        .await
        .unwrap();

    let err = WorkOrderRepo::transition(
        &pool,
        wo.id,
        &TransitionRequest {
            tenant_id: tid.clone(),
            status: "in_progress".into(),
            completed_at: None,
            downtime_minutes: None,
            closed_at: None,
            notes: None,
        },
    )
    .await
    .unwrap_err();

    assert!(
        matches!(err, WoError::Transition(_)),
        "expected Transition error, got: {:?}",
        err
    );
}

// ============================================================================
// 3. Completion guard
// ============================================================================

#[tokio::test]
#[serial]
async fn test_completion_requires_guard_fields() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let asset_id = create_test_asset(&pool, &tid, "GUARD-001").await;

    let wo = WorkOrderRepo::create(&pool, &base_create_req(&tid, asset_id))
        .await
        .unwrap();

    transition_simple(&pool, wo.id, &tid, "scheduled").await;
    transition_simple(&pool, wo.id, &tid, "in_progress").await;

    let err = WorkOrderRepo::transition(
        &pool,
        wo.id,
        &TransitionRequest {
            tenant_id: tid.clone(),
            status: "completed".into(),
            completed_at: None,
            downtime_minutes: None,
            closed_at: None,
            notes: None,
        },
    )
    .await
    .unwrap_err();

    assert!(
        matches!(err, WoError::Guard(_)),
        "expected Guard error, got: {:?}",
        err
    );
}

// ============================================================================
// 4. Close guard
// ============================================================================

#[tokio::test]
#[serial]
async fn test_close_requires_closed_at() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let asset_id = create_test_asset(&pool, &tid, "CLOSE-001").await;

    let wo = WorkOrderRepo::create(&pool, &base_create_req(&tid, asset_id))
        .await
        .unwrap();

    transition_simple(&pool, wo.id, &tid, "scheduled").await;
    transition_simple(&pool, wo.id, &tid, "in_progress").await;

    let now = chrono::Utc::now();
    WorkOrderRepo::transition(
        &pool,
        wo.id,
        &TransitionRequest {
            tenant_id: tid.clone(),
            status: "completed".into(),
            completed_at: Some(now),
            downtime_minutes: Some(10),
            closed_at: None,
            notes: None,
        },
    )
    .await
    .unwrap();

    let err = WorkOrderRepo::transition(
        &pool,
        wo.id,
        &TransitionRequest {
            tenant_id: tid.clone(),
            status: "closed".into(),
            completed_at: None,
            downtime_minutes: None,
            closed_at: None,
            notes: None,
        },
    )
    .await
    .unwrap_err();

    assert!(
        matches!(err, WoError::Guard(_)),
        "expected Guard error, got: {:?}",
        err
    );
}

// ============================================================================
// 5. Cancel from draft
// ============================================================================

#[tokio::test]
#[serial]
async fn test_cancel_from_draft() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let asset_id = create_test_asset(&pool, &tid, "CXL-001").await;

    let wo = WorkOrderRepo::create(&pool, &base_create_req(&tid, asset_id))
        .await
        .unwrap();

    let wo = WorkOrderRepo::transition(
        &pool,
        wo.id,
        &TransitionRequest {
            tenant_id: tid.clone(),
            status: "cancelled".into(),
            completed_at: None,
            downtime_minutes: None,
            closed_at: None,
            notes: Some("No longer needed".into()),
        },
    )
    .await
    .unwrap();
    assert_eq!(wo.status.as_str(), "cancelled");

    let event: Option<(String,)> = sqlx::query_as(
        r#"SELECT event_type FROM events_outbox
           WHERE aggregate_id = $1
             AND event_type = 'maintenance.work_order.cancelled'"#,
    )
    .bind(wo.id.to_string())
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert!(event.is_some());
}

// ============================================================================
// 6. Cannot transition from terminal
// ============================================================================

#[tokio::test]
#[serial]
async fn test_cannot_transition_from_terminal() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let asset_id = create_test_asset(&pool, &tid, "TERM-001").await;

    let wo = WorkOrderRepo::create(&pool, &base_create_req(&tid, asset_id))
        .await
        .unwrap();

    WorkOrderRepo::transition(
        &pool,
        wo.id,
        &TransitionRequest {
            tenant_id: tid.clone(),
            status: "cancelled".into(),
            completed_at: None,
            downtime_minutes: None,
            closed_at: None,
            notes: None,
        },
    )
    .await
    .unwrap();

    let err = WorkOrderRepo::transition(
        &pool,
        wo.id,
        &TransitionRequest {
            tenant_id: tid,
            status: "draft".into(),
            completed_at: None,
            downtime_minutes: None,
            closed_at: None,
            notes: None,
        },
    )
    .await
    .unwrap_err();

    assert!(
        matches!(err, WoError::Transition(_)),
        "expected Transition error, got: {:?}",
        err
    );
}
