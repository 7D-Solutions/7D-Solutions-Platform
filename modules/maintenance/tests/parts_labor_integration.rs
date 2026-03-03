//! Integration tests for work order parts and labor subresources (bd-260x).
//!
//! Covers:
//! 1. Add parts to in_progress WO — succeeds
//! 2. List parts for a WO
//! 3. Remove part from WO
//! 4. Add labor to in_progress WO — succeeds
//! 5. List labor for a WO
//! 6. Remove labor from WO
//! 7. Cannot add part after WO completed
//! 8. Cannot add labor after WO closed
//! 9. Cannot remove part after WO cancelled

use maintenance_rs::domain::assets::{AssetRepo, CreateAssetRequest};
use maintenance_rs::domain::work_orders::{
    AddLaborRequest, AddPartRequest, CreateWorkOrderRequest, TransitionRequest, WoLaborError,
    WoLaborRepo, WoPartError, WoPartsRepo, WorkOrderRepo,
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
    format!("pl-test-{}", Uuid::new_v4().simple())
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

/// Create a WO and advance it to in_progress.
async fn create_in_progress_wo(pool: &sqlx::PgPool, tid: &str, asset_id: Uuid) -> Uuid {
    let wo = WorkOrderRepo::create(
        pool,
        &CreateWorkOrderRequest {
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
        },
    )
    .await
    .unwrap();

    // draft → scheduled → in_progress
    transition_simple(pool, wo.id, tid, "scheduled").await;
    transition_simple(pool, wo.id, tid, "in_progress").await;
    wo.id
}

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

fn base_part_req(tid: &str) -> AddPartRequest {
    AddPartRequest {
        tenant_id: tid.to_string(),
        part_description: "Oil filter".into(),
        part_ref: Some("SKU-OIL-001".into()),
        quantity: 2,
        unit_cost_minor: 1500,
        currency: None,
        inventory_issue_ref: None,
    }
}

fn base_labor_req(tid: &str) -> AddLaborRequest {
    AddLaborRequest {
        tenant_id: tid.to_string(),
        technician_ref: "tech-42".into(),
        hours_decimal: "2.50".into(),
        rate_minor: 7500,
        currency: None,
        description: Some("Oil change labor".into()),
    }
}

// ============================================================================
// 1. Add parts to in_progress WO
// ============================================================================

#[tokio::test]
#[serial]
async fn test_add_part_to_in_progress_wo() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let asset_id = create_test_asset(&pool, &tid, "PART-001").await;
    let wo_id = create_in_progress_wo(&pool, &tid, asset_id).await;

    let part = WoPartsRepo::add(&pool, wo_id, &base_part_req(&tid))
        .await
        .unwrap();

    assert_eq!(part.tenant_id, tid);
    assert_eq!(part.work_order_id, wo_id);
    assert_eq!(part.part_description, "Oil filter");
    assert_eq!(part.part_ref, Some("SKU-OIL-001".into()));
    assert_eq!(part.quantity, 2);
    assert_eq!(part.unit_cost_minor, 1500);
    assert_eq!(part.currency, "USD");
    assert!(part.inventory_issue_ref.is_none());
}

// ============================================================================
// 2. List parts for a WO
// ============================================================================

#[tokio::test]
#[serial]
async fn test_list_parts() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let asset_id = create_test_asset(&pool, &tid, "PART-002").await;
    let wo_id = create_in_progress_wo(&pool, &tid, asset_id).await;

    WoPartsRepo::add(&pool, wo_id, &base_part_req(&tid))
        .await
        .unwrap();

    let mut req2 = base_part_req(&tid);
    req2.part_description = "Air filter".into();
    req2.quantity = 1;
    WoPartsRepo::add(&pool, wo_id, &req2).await.unwrap();

    let parts = WoPartsRepo::list(&pool, wo_id, &tid).await.unwrap();
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0].part_description, "Oil filter");
    assert_eq!(parts[1].part_description, "Air filter");
}

// ============================================================================
// 3. Remove part from WO
// ============================================================================

#[tokio::test]
#[serial]
async fn test_remove_part() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let asset_id = create_test_asset(&pool, &tid, "PART-003").await;
    let wo_id = create_in_progress_wo(&pool, &tid, asset_id).await;

    let part = WoPartsRepo::add(&pool, wo_id, &base_part_req(&tid))
        .await
        .unwrap();

    WoPartsRepo::remove(&pool, wo_id, part.id, &tid)
        .await
        .unwrap();

    let parts = WoPartsRepo::list(&pool, wo_id, &tid).await.unwrap();
    assert!(parts.is_empty());
}

// ============================================================================
// 4. Add labor to in_progress WO
// ============================================================================

#[tokio::test]
#[serial]
async fn test_add_labor_to_in_progress_wo() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let asset_id = create_test_asset(&pool, &tid, "LABOR-001").await;
    let wo_id = create_in_progress_wo(&pool, &tid, asset_id).await;

    let labor = WoLaborRepo::add(&pool, wo_id, &base_labor_req(&tid))
        .await
        .unwrap();

    assert_eq!(labor.tenant_id, tid);
    assert_eq!(labor.work_order_id, wo_id);
    assert_eq!(labor.technician_ref, "tech-42");
    assert_eq!(labor.hours_decimal.to_string(), "2.50");
    assert_eq!(labor.rate_minor, 7500);
    assert_eq!(labor.currency, "USD");
    assert_eq!(labor.description, Some("Oil change labor".into()));
}

// ============================================================================
// 5. List labor for a WO
// ============================================================================

#[tokio::test]
#[serial]
async fn test_list_labor() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let asset_id = create_test_asset(&pool, &tid, "LABOR-002").await;
    let wo_id = create_in_progress_wo(&pool, &tid, asset_id).await;

    WoLaborRepo::add(&pool, wo_id, &base_labor_req(&tid))
        .await
        .unwrap();

    let mut req2 = base_labor_req(&tid);
    req2.technician_ref = "tech-99".into();
    req2.hours_decimal = "1.00".into();
    WoLaborRepo::add(&pool, wo_id, &req2).await.unwrap();

    let entries = WoLaborRepo::list(&pool, wo_id, &tid).await.unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].technician_ref, "tech-42");
    assert_eq!(entries[1].technician_ref, "tech-99");
}

// ============================================================================
// 6. Remove labor from WO
// ============================================================================

#[tokio::test]
#[serial]
async fn test_remove_labor() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let asset_id = create_test_asset(&pool, &tid, "LABOR-003").await;
    let wo_id = create_in_progress_wo(&pool, &tid, asset_id).await;

    let labor = WoLaborRepo::add(&pool, wo_id, &base_labor_req(&tid))
        .await
        .unwrap();

    WoLaborRepo::remove(&pool, wo_id, labor.id, &tid)
        .await
        .unwrap();

    let entries = WoLaborRepo::list(&pool, wo_id, &tid).await.unwrap();
    assert!(entries.is_empty());
}

// ============================================================================
// 7. Cannot add part after WO completed
// ============================================================================

#[tokio::test]
#[serial]
async fn test_cannot_add_part_after_completed() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let asset_id = create_test_asset(&pool, &tid, "IMMUT-001").await;
    let wo_id = create_in_progress_wo(&pool, &tid, asset_id).await;

    // in_progress → completed
    let now = chrono::Utc::now();
    WorkOrderRepo::transition(
        &pool,
        wo_id,
        &TransitionRequest {
            tenant_id: tid.clone(),
            status: "completed".into(),
            completed_at: Some(now),
            downtime_minutes: Some(30),
            closed_at: None,
            notes: None,
        },
    )
    .await
    .unwrap();

    let err = WoPartsRepo::add(&pool, wo_id, &base_part_req(&tid))
        .await
        .unwrap_err();
    assert!(
        matches!(err, WoPartError::WoImmutable(_)),
        "expected WoImmutable, got: {:?}",
        err
    );
}

// ============================================================================
// 8. Cannot add labor after WO closed
// ============================================================================

#[tokio::test]
#[serial]
async fn test_cannot_add_labor_after_closed() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let asset_id = create_test_asset(&pool, &tid, "IMMUT-002").await;
    let wo_id = create_in_progress_wo(&pool, &tid, asset_id).await;

    let now = chrono::Utc::now();
    // in_progress → completed → closed
    WorkOrderRepo::transition(
        &pool,
        wo_id,
        &TransitionRequest {
            tenant_id: tid.clone(),
            status: "completed".into(),
            completed_at: Some(now),
            downtime_minutes: Some(30),
            closed_at: None,
            notes: None,
        },
    )
    .await
    .unwrap();

    WorkOrderRepo::transition(
        &pool,
        wo_id,
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

    let err = WoLaborRepo::add(&pool, wo_id, &base_labor_req(&tid))
        .await
        .unwrap_err();
    assert!(
        matches!(err, WoLaborError::WoImmutable(_)),
        "expected WoImmutable, got: {:?}",
        err
    );
}

// ============================================================================
// 9. Cannot remove part after WO cancelled
// ============================================================================

#[tokio::test]
#[serial]
async fn test_cannot_remove_part_after_cancelled() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let asset_id = create_test_asset(&pool, &tid, "IMMUT-003").await;
    let wo_id = create_in_progress_wo(&pool, &tid, asset_id).await;

    // Add part while in_progress
    let part = WoPartsRepo::add(&pool, wo_id, &base_part_req(&tid))
        .await
        .unwrap();

    // in_progress → cancelled
    WorkOrderRepo::transition(
        &pool,
        wo_id,
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

    let err = WoPartsRepo::remove(&pool, wo_id, part.id, &tid)
        .await
        .unwrap_err();
    assert!(
        matches!(err, WoPartError::WoImmutable(_)),
        "expected WoImmutable, got: {:?}",
        err
    );

    // But listing should still work
    let parts = WoPartsRepo::list(&pool, wo_id, &tid).await.unwrap();
    assert_eq!(parts.len(), 1);
}
