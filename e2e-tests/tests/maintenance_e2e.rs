//! E2E tests: Maintenance module — work orders, schedules, asset integration
//!
//! Covers:
//! 1. Asset CRUD lifecycle (create, get, list, update)
//! 2. Work order creation linked to asset + full state machine lifecycle
//! 3. Preventive maintenance plan creation, retrieval, and assignment to asset
//! 4. Invalid state transitions rejected by state machine
//! 5. Work order filtered listing by asset and status
//!
//! All tests hit real Postgres (maintenance DB port 5452). No mocks, no stubs.

use chrono::Utc;
use maintenance_rs::domain::{
    assets::{AssetRepo, CreateAssetRequest, UpdateAssetRequest},
    plans::{AssignPlanRequest, AssignmentRepo, CreatePlanRequest, ListAssignmentsQuery, PlanRepo},
    work_orders::{CreateWorkOrderRequest, ListWorkOrdersQuery, TransitionRequest, WorkOrderRepo},
};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::time::Duration;
use uuid::Uuid;

// ============================================================================
// DB setup
// ============================================================================

async fn maint_pool() -> PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("MAINTENANCE_DATABASE_URL").unwrap_or_else(|_| {
        "postgres://maintenance_user:maintenance_pass@localhost:5452/maintenance_db?sslmode=require"
            .to_string()
    });
    let pool = PgPoolOptions::new()
        .max_connections(3)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&url)
        .await
        .expect("Failed to connect to maintenance DB");

    sqlx::migrate!("../modules/maintenance/db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run maintenance migrations");

    pool
}

fn tenant() -> String {
    format!("maint-e2e-{}", Uuid::new_v4())
}

/// Helper: create an asset and return its ID.
async fn create_test_asset(pool: &PgPool, tenant_id: &str) -> Uuid {
    let req = CreateAssetRequest {
        tenant_id: tenant_id.to_string(),
        asset_tag: format!("TAG-{}", Uuid::new_v4().as_simple()),
        name: "CNC Mill #7".to_string(),
        description: Some("Haas VF-2 vertical machining center".to_string()),
        asset_type: "machinery".to_string(),
        location: Some("Building A, Bay 3".to_string()),
        department: Some("Manufacturing".to_string()),
        responsible_person: Some("J. Smith".to_string()),
        serial_number: Some(format!("SN-{}", Uuid::new_v4().as_simple())),
        fixed_asset_ref: None,
        metadata: None,
        maintenance_schedule: None,
        idempotency_key: Some(Uuid::new_v4().to_string()),
    };
    let asset = AssetRepo::create(pool, &req).await.expect("create asset");
    asset.id
}

// ============================================================================
// Test 1: Asset CRUD lifecycle — create, get, list, update
// ============================================================================

#[tokio::test]
#[serial]
async fn asset_crud_lifecycle() {
    let pool = maint_pool().await;
    let t = tenant();

    // Create
    let req = CreateAssetRequest {
        tenant_id: t.clone(),
        asset_tag: format!("ASSET-{}", Uuid::new_v4().as_simple()),
        name: "Hydraulic Press #2".to_string(),
        description: Some("500-ton hydraulic forming press".to_string()),
        asset_type: "machinery".to_string(),
        location: Some("Shop Floor West".to_string()),
        department: Some("Fabrication".to_string()),
        responsible_person: Some("M. Johnson".to_string()),
        serial_number: Some("HP-2026-0042".to_string()),
        fixed_asset_ref: None,
        metadata: None,
        maintenance_schedule: None,
        idempotency_key: Some(Uuid::new_v4().to_string()),
    };

    let asset = AssetRepo::create(&pool, &req).await.expect("create asset");
    assert_eq!(asset.tenant_id, t);
    assert_eq!(asset.name, "Hydraulic Press #2");
    assert_eq!(asset.asset_type.as_str(), "machinery");
    assert_eq!(asset.status.as_str(), "active");

    // Get by ID
    let fetched = AssetRepo::find_by_id(&pool, asset.id, &t)
        .await
        .expect("find asset")
        .expect("asset should exist");
    assert_eq!(fetched.id, asset.id);
    assert_eq!(fetched.name, "Hydraulic Press #2");

    // List
    let list = AssetRepo::list(
        &pool,
        &maintenance_rs::domain::assets::ListAssetsQuery {
            tenant_id: t.clone(),
            asset_type: Some("machinery".to_string()),
            status: None,
            limit: Some(10),
            offset: None,
        },
    )
    .await
    .expect("list assets");
    assert!(list.total >= 1);
    assert!(list.items.iter().any(|a| a.id == asset.id));

    // Update
    let update_req = UpdateAssetRequest {
        name: Some("Hydraulic Press #2 (Refurbished)".to_string()),
        description: None,
        asset_type: None,
        location: Some("Shop Floor East".to_string()),
        department: None,
        responsible_person: None,
        serial_number: None,
        fixed_asset_ref: None,
        status: None,
        metadata: None,
        maintenance_schedule: None,
        out_of_service: None,
        out_of_service_reason: None,
    };
    let updated = AssetRepo::update(&pool, asset.id, &t, &update_req)
        .await
        .expect("update asset");
    assert_eq!(updated.name, "Hydraulic Press #2 (Refurbished)");
    assert_eq!(updated.location.as_deref(), Some("Shop Floor East"));
}

// ============================================================================
// Test 2: Work order full lifecycle — draft → scheduled → in_progress →
//         completed → closed
// ============================================================================

#[tokio::test]
#[serial]
async fn work_order_full_lifecycle() {
    let pool = maint_pool().await;
    let t = tenant();
    let asset_id = create_test_asset(&pool, &t).await;

    // Create work order
    let req = CreateWorkOrderRequest {
        tenant_id: t.clone(),
        asset_id,
        plan_assignment_id: None,
        title: "Annual bearing replacement".to_string(),
        description: Some("Replace main spindle bearings per OEM schedule".to_string()),
        wo_type: "preventive".to_string(),
        priority: Some("high".to_string()),
        assigned_to: Some("Tech-A".to_string()),
        scheduled_date: None,
        checklist: None,
        notes: None,
    };

    let wo = WorkOrderRepo::create(&pool, &req).await.expect("create WO");
    assert_eq!(wo.tenant_id, t);
    assert_eq!(wo.asset_id, asset_id);
    assert_eq!(wo.status.as_str(), "draft");
    assert_eq!(wo.wo_type.as_str(), "preventive");
    assert_eq!(wo.priority.as_str(), "high");
    assert!(wo.wo_number.starts_with("WO-"));

    // Get by ID
    let fetched = WorkOrderRepo::find_by_id(&pool, wo.id, &t)
        .await
        .expect("find WO")
        .expect("WO should exist");
    assert_eq!(fetched.id, wo.id);
    assert_eq!(fetched.title, "Annual bearing replacement");

    // Draft → Scheduled
    let scheduled = WorkOrderRepo::transition(
        &pool,
        wo.id,
        &TransitionRequest {
            tenant_id: t.clone(),
            status: "scheduled".to_string(),
            completed_at: None,
            downtime_minutes: None,
            closed_at: None,
            notes: None,
        },
    )
    .await
    .expect("transition to scheduled");
    assert_eq!(scheduled.status.as_str(), "scheduled");

    // Scheduled → In Progress
    let in_progress = WorkOrderRepo::transition(
        &pool,
        wo.id,
        &TransitionRequest {
            tenant_id: t.clone(),
            status: "in_progress".to_string(),
            completed_at: None,
            downtime_minutes: None,
            closed_at: None,
            notes: None,
        },
    )
    .await
    .expect("transition to in_progress");
    assert_eq!(in_progress.status.as_str(), "in_progress");
    assert!(in_progress.started_at.is_some(), "started_at should be set");

    // In Progress → Completed (requires completed_at + downtime_minutes)
    let now = Utc::now();
    let completed = WorkOrderRepo::transition(
        &pool,
        wo.id,
        &TransitionRequest {
            tenant_id: t.clone(),
            status: "completed".to_string(),
            completed_at: Some(now),
            downtime_minutes: Some(120),
            closed_at: None,
            notes: Some("Bearings replaced, test run OK".to_string()),
        },
    )
    .await
    .expect("transition to completed");
    assert_eq!(completed.status.as_str(), "completed");
    assert!(completed.completed_at.is_some());
    assert_eq!(completed.downtime_minutes, Some(120));

    // Completed → Closed (requires closed_at)
    let closed = WorkOrderRepo::transition(
        &pool,
        wo.id,
        &TransitionRequest {
            tenant_id: t.clone(),
            status: "closed".to_string(),
            completed_at: None,
            downtime_minutes: None,
            closed_at: Some(Utc::now()),
            notes: None,
        },
    )
    .await
    .expect("transition to closed");
    assert_eq!(closed.status.as_str(), "closed");
    assert!(closed.closed_at.is_some());

    // Verify outbox events were created
    let events: Vec<(String,)> =
        sqlx::query_as("SELECT event_type FROM events_outbox ORDER BY created_at")
            .fetch_all(&pool)
            .await
            .unwrap();

    let types: Vec<&str> = events.iter().map(|r| r.0.as_str()).collect();
    assert!(
        types.contains(&"maintenance.asset.created"),
        "should have asset.created event"
    );
    assert!(
        types.contains(&"maintenance.work_order.created"),
        "should have work_order.created event"
    );
    assert!(
        types.contains(&"maintenance.work_order.completed"),
        "should have work_order.completed event"
    );
    assert!(
        types.contains(&"maintenance.work_order.closed"),
        "should have work_order.closed event"
    );
}

// ============================================================================
// Test 3: Preventive maintenance plan — create, get, assign to asset
// ============================================================================

#[tokio::test]
#[serial]
async fn plan_create_get_assign() {
    let pool = maint_pool().await;
    let t = tenant();
    let asset_id = create_test_asset(&pool, &t).await;

    // Create plan
    let req = CreatePlanRequest {
        tenant_id: t.clone(),
        name: "90-Day Lubrication Service".to_string(),
        description: Some("Grease all bearings per OEM spec".to_string()),
        asset_type_filter: Some("machinery".to_string()),
        schedule_type: "calendar".to_string(),
        calendar_interval_days: Some(90),
        meter_type_id: None,
        meter_interval: None,
        priority: Some("medium".to_string()),
        estimated_duration_minutes: Some(60),
        estimated_cost_minor: Some(15_000), // $150.00
        task_checklist: Some(serde_json::json!([
            {"step": 1, "task": "Lock out / tag out"},
            {"step": 2, "task": "Grease main bearings"},
            {"step": 3, "task": "Inspect seals"},
        ])),
    };

    let plan = PlanRepo::create(&pool, &req).await.expect("create plan");
    assert_eq!(plan.tenant_id, t);
    assert_eq!(plan.name, "90-Day Lubrication Service");
    assert_eq!(plan.schedule_type.as_str(), "calendar");
    assert_eq!(plan.calendar_interval_days, Some(90));
    assert_eq!(plan.priority.as_str(), "medium");
    assert!(plan.is_active);

    // Get plan by ID
    let fetched = PlanRepo::find_by_id(&pool, plan.id, &t)
        .await
        .expect("find plan")
        .expect("plan should exist");
    assert_eq!(fetched.id, plan.id);
    assert_eq!(fetched.name, "90-Day Lubrication Service");

    // Assign plan to asset
    let assign_req = AssignPlanRequest {
        tenant_id: t.clone(),
        asset_id,
    };
    let assignment = AssignmentRepo::assign(&pool, plan.id, &assign_req)
        .await
        .expect("assign plan");
    assert_eq!(assignment.plan_id, plan.id);
    assert_eq!(assignment.asset_id, asset_id);
    assert_eq!(assignment.tenant_id, t);
    assert!(
        assignment.next_due_date.is_some(),
        "calendar plan should compute next_due_date"
    );

    // List assignments by plan
    let assignments = AssignmentRepo::list(
        &pool,
        &ListAssignmentsQuery {
            tenant_id: t.clone(),
            plan_id: Some(plan.id),
            asset_id: None,
            limit: Some(10),
            offset: None,
        },
    )
    .await
    .expect("list assignments");
    assert_eq!(assignments.len(), 1);
    assert_eq!(assignments[0].plan_id, plan.id);
    assert_eq!(assignments[0].asset_id, asset_id);

    // List assignments by asset
    let by_asset = AssignmentRepo::list(
        &pool,
        &ListAssignmentsQuery {
            tenant_id: t.clone(),
            plan_id: None,
            asset_id: Some(asset_id),
            limit: Some(10),
            offset: None,
        },
    )
    .await
    .expect("list assignments by asset");
    assert_eq!(by_asset.len(), 1);
}

// ============================================================================
// Test 4: Invalid state transitions — state machine rejects illegal moves
// ============================================================================

#[tokio::test]
#[serial]
async fn invalid_state_transitions_rejected() {
    let pool = maint_pool().await;
    let t = tenant();
    let asset_id = create_test_asset(&pool, &t).await;

    let req = CreateWorkOrderRequest {
        tenant_id: t.clone(),
        asset_id,
        plan_assignment_id: None,
        title: "Test invalid transitions".to_string(),
        description: None,
        wo_type: "corrective".to_string(),
        priority: None,
        assigned_to: None,
        scheduled_date: None,
        checklist: None,
        notes: None,
    };

    let wo = WorkOrderRepo::create(&pool, &req).await.expect("create WO");
    assert_eq!(wo.status.as_str(), "draft");

    // Draft → InProgress should fail (must go through scheduled)
    let err = WorkOrderRepo::transition(
        &pool,
        wo.id,
        &TransitionRequest {
            tenant_id: t.clone(),
            status: "in_progress".to_string(),
            completed_at: None,
            downtime_minutes: None,
            closed_at: None,
            notes: None,
        },
    )
    .await;
    assert!(err.is_err(), "Draft → InProgress must be rejected");

    // Draft → Completed should fail
    let err = WorkOrderRepo::transition(
        &pool,
        wo.id,
        &TransitionRequest {
            tenant_id: t.clone(),
            status: "completed".to_string(),
            completed_at: Some(Utc::now()),
            downtime_minutes: Some(0),
            closed_at: None,
            notes: None,
        },
    )
    .await;
    assert!(err.is_err(), "Draft → Completed must be rejected");

    // Draft → Closed should fail
    let err = WorkOrderRepo::transition(
        &pool,
        wo.id,
        &TransitionRequest {
            tenant_id: t.clone(),
            status: "closed".to_string(),
            completed_at: None,
            downtime_minutes: None,
            closed_at: Some(Utc::now()),
            notes: None,
        },
    )
    .await;
    assert!(err.is_err(), "Draft → Closed must be rejected");

    // Move to scheduled, then in_progress, then try completing WITHOUT required fields
    WorkOrderRepo::transition(
        &pool,
        wo.id,
        &TransitionRequest {
            tenant_id: t.clone(),
            status: "scheduled".to_string(),
            completed_at: None,
            downtime_minutes: None,
            closed_at: None,
            notes: None,
        },
    )
    .await
    .expect("scheduled");

    WorkOrderRepo::transition(
        &pool,
        wo.id,
        &TransitionRequest {
            tenant_id: t.clone(),
            status: "in_progress".to_string(),
            completed_at: None,
            downtime_minutes: None,
            closed_at: None,
            notes: None,
        },
    )
    .await
    .expect("in_progress");

    // InProgress → Completed without completed_at should fail (guard)
    let err = WorkOrderRepo::transition(
        &pool,
        wo.id,
        &TransitionRequest {
            tenant_id: t.clone(),
            status: "completed".to_string(),
            completed_at: None,
            downtime_minutes: Some(30),
            closed_at: None,
            notes: None,
        },
    )
    .await;
    assert!(
        err.is_err(),
        "Completing without completed_at must be rejected"
    );

    // InProgress → Completed without downtime_minutes should fail (guard)
    let err = WorkOrderRepo::transition(
        &pool,
        wo.id,
        &TransitionRequest {
            tenant_id: t.clone(),
            status: "completed".to_string(),
            completed_at: Some(Utc::now()),
            downtime_minutes: None,
            closed_at: None,
            notes: None,
        },
    )
    .await;
    assert!(
        err.is_err(),
        "Completing without downtime_minutes must be rejected"
    );

    // Cancel from in_progress (should succeed)
    let cancelled = WorkOrderRepo::transition(
        &pool,
        wo.id,
        &TransitionRequest {
            tenant_id: t.clone(),
            status: "cancelled".to_string(),
            completed_at: None,
            downtime_minutes: None,
            closed_at: None,
            notes: Some("Test cancellation".to_string()),
        },
    )
    .await
    .expect("cancel");
    assert_eq!(cancelled.status.as_str(), "cancelled");

    // Cancelled is terminal — any transition should fail
    let err = WorkOrderRepo::transition(
        &pool,
        wo.id,
        &TransitionRequest {
            tenant_id: t.clone(),
            status: "draft".to_string(),
            completed_at: None,
            downtime_minutes: None,
            closed_at: None,
            notes: None,
        },
    )
    .await;
    assert!(err.is_err(), "Cancelled is terminal — no transitions out");
}

// ============================================================================
// Test 5: Work order list filtering by asset and status
// ============================================================================

#[tokio::test]
#[serial]
async fn work_order_list_filtering() {
    let pool = maint_pool().await;
    let t = tenant();
    let asset_a = create_test_asset(&pool, &t).await;
    let asset_b = create_test_asset(&pool, &t).await;

    // Create 2 WOs for asset_a, 1 for asset_b
    for (asset_id, title) in [(asset_a, "WO-A1"), (asset_a, "WO-A2"), (asset_b, "WO-B1")] {
        WorkOrderRepo::create(
            &pool,
            &CreateWorkOrderRequest {
                tenant_id: t.clone(),
                asset_id,
                plan_assignment_id: None,
                title: title.to_string(),
                description: None,
                wo_type: "corrective".to_string(),
                priority: None,
                assigned_to: None,
                scheduled_date: None,
                checklist: None,
                notes: None,
            },
        )
        .await
        .expect("create WO");
    }

    // Filter by asset_a
    let by_asset_a = WorkOrderRepo::list(
        &pool,
        &ListWorkOrdersQuery {
            tenant_id: t.clone(),
            asset_id: Some(asset_a),
            status: None,
            limit: Some(50),
            offset: None,
        },
    )
    .await
    .expect("list by asset_a");
    assert_eq!(by_asset_a.len(), 2);

    // Filter by asset_b
    let by_asset_b = WorkOrderRepo::list(
        &pool,
        &ListWorkOrdersQuery {
            tenant_id: t.clone(),
            asset_id: Some(asset_b),
            status: None,
            limit: Some(50),
            offset: None,
        },
    )
    .await
    .expect("list by asset_b");
    assert_eq!(by_asset_b.len(), 1);

    // Filter by status=draft (all 3 should be draft)
    let by_draft = WorkOrderRepo::list(
        &pool,
        &ListWorkOrdersQuery {
            tenant_id: t.clone(),
            asset_id: None,
            status: Some("draft".to_string()),
            limit: Some(50),
            offset: None,
        },
    )
    .await
    .expect("list by draft");
    assert!(by_draft.len() >= 3);

    // Transition one to scheduled, then filter
    let wo_id = by_asset_a[0].id;
    WorkOrderRepo::transition(
        &pool,
        wo_id,
        &TransitionRequest {
            tenant_id: t.clone(),
            status: "scheduled".to_string(),
            completed_at: None,
            downtime_minutes: None,
            closed_at: None,
            notes: None,
        },
    )
    .await
    .expect("schedule");

    let by_scheduled = WorkOrderRepo::list(
        &pool,
        &ListWorkOrdersQuery {
            tenant_id: t.clone(),
            asset_id: Some(asset_a),
            status: Some("scheduled".to_string()),
            limit: Some(50),
            offset: None,
        },
    )
    .await
    .expect("list scheduled");
    assert_eq!(by_scheduled.len(), 1);
    assert_eq!(by_scheduled[0].id, wo_id);
}
