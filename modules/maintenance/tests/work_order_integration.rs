//! Integration tests for work order CRUD (bd-3nvm).
//!
//! Covers:
//! 1. Create ad-hoc work order — WO number allocated, status=draft, event in outbox
//! 2. WO numbers sequential within tenant
//! 3. List with status filter
//! 4. Tenant isolation (get + transition)

use maintenance_rs::domain::assets::{AssetRepo, CreateAssetRequest};
use maintenance_rs::domain::work_orders::{
    CreateWorkOrderRequest, ListWorkOrdersQuery, TransitionRequest, WoError, WorkOrderRepo,
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
    format!("wo-test-{}", Uuid::new_v4().simple())
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

// ============================================================================
// 1. Create ad-hoc work order
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_adhoc_work_order() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let asset_id = create_test_asset(&pool, &tid, "WO-001").await;

    let wo = WorkOrderRepo::create(&pool, &base_create_req(&tid, asset_id))
        .await
        .unwrap();

    assert_eq!(wo.tenant_id, tid);
    assert_eq!(wo.asset_id, asset_id);
    assert_eq!(wo.wo_number, "WO-000001");
    assert_eq!(wo.status.as_str(), "draft");
    assert_eq!(wo.wo_type.as_str(), "corrective");
    assert_eq!(wo.priority.as_str(), "medium");
    assert!(wo.plan_assignment_id.is_none());

    // Verify outbox event
    let event: Option<(String,)> = sqlx::query_as(
        "SELECT event_type FROM events_outbox WHERE aggregate_id = $1",
    )
    .bind(wo.id.to_string())
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert_eq!(event.unwrap().0, "maintenance.work_order.created");
}

// ============================================================================
// 2. WO numbers sequential within tenant
// ============================================================================

#[tokio::test]
#[serial]
async fn test_wo_numbers_sequential() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let asset_id = create_test_asset(&pool, &tid, "SEQ-001").await;

    let wo1 = WorkOrderRepo::create(&pool, &base_create_req(&tid, asset_id))
        .await
        .unwrap();
    let wo2 = WorkOrderRepo::create(&pool, &base_create_req(&tid, asset_id))
        .await
        .unwrap();
    let wo3 = WorkOrderRepo::create(&pool, &base_create_req(&tid, asset_id))
        .await
        .unwrap();

    assert_eq!(wo1.wo_number, "WO-000001");
    assert_eq!(wo2.wo_number, "WO-000002");
    assert_eq!(wo3.wo_number, "WO-000003");
}

// ============================================================================
// 3. List with status filter
// ============================================================================

#[tokio::test]
#[serial]
async fn test_list_with_status_filter() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let asset_id = create_test_asset(&pool, &tid, "LIST-001").await;

    let wo1 = WorkOrderRepo::create(&pool, &base_create_req(&tid, asset_id))
        .await
        .unwrap();
    WorkOrderRepo::create(&pool, &base_create_req(&tid, asset_id))
        .await
        .unwrap();

    // Transition first to scheduled
    WorkOrderRepo::transition(
        &pool,
        wo1.id,
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

    let all = WorkOrderRepo::list(
        &pool,
        &ListWorkOrdersQuery {
            tenant_id: tid.clone(),
            asset_id: None,
            status: None,
            limit: None,
            offset: None,
        },
    )
    .await
    .unwrap();
    assert!(all.len() >= 2);

    let drafts = WorkOrderRepo::list(
        &pool,
        &ListWorkOrdersQuery {
            tenant_id: tid.clone(),
            asset_id: None,
            status: Some("draft".into()),
            limit: None,
            offset: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(drafts.len(), 1);

    let scheduled = WorkOrderRepo::list(
        &pool,
        &ListWorkOrdersQuery {
            tenant_id: tid,
            asset_id: None,
            status: Some("scheduled".into()),
            limit: None,
            offset: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(scheduled.len(), 1);
}

// ============================================================================
// 4. Tenant isolation
// ============================================================================

#[tokio::test]
#[serial]
async fn test_tenant_isolation() {
    let pool = setup_db().await;
    let tid_a = unique_tenant();
    let tid_b = unique_tenant();
    let asset_a = create_test_asset(&pool, &tid_a, "ISO-A").await;
    let asset_b = create_test_asset(&pool, &tid_b, "ISO-B").await;

    let wo_a = WorkOrderRepo::create(&pool, &base_create_req(&tid_a, asset_a))
        .await
        .unwrap();
    WorkOrderRepo::create(&pool, &base_create_req(&tid_b, asset_b))
        .await
        .unwrap();

    // Tenant B cannot see tenant A's WO
    let result = WorkOrderRepo::find_by_id(&pool, wo_a.id, &tid_b)
        .await
        .unwrap();
    assert!(result.is_none());

    // Tenant B cannot transition tenant A's WO
    let err = WorkOrderRepo::transition(
        &pool,
        wo_a.id,
        &TransitionRequest {
            tenant_id: tid_b,
            status: "scheduled".into(),
            completed_at: None,
            downtime_minutes: None,
            closed_at: None,
            notes: None,
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, WoError::NotFound));
}
