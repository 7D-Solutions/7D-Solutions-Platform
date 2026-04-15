//! Audit Oracle — Production module
//!
//! Asserts that every Production mutation writes exactly one audit_events row
//! inside the same transaction as the mutation.
//!
//! Covered mutations:
//!   - WorkOrderRepo::create   → CreateWorkOrder   (CREATE)
//!   - WorkOrderRepo::release  → ReleaseWorkOrder  (STATE_TRANSITION)
//!   - WorkOrderRepo::close    → CloseWorkOrder    (STATE_TRANSITION)
//!   - request_component_issue → RequestComponentIssue (CREATE)
//!   - request_fg_receipt      → RequestFgReceipt  (CREATE)
//!
//! Real database, no mocks. Run:
//!   ./scripts/cargo-slot.sh test -p production-rs audit_oracle -- --nocapture

use production_rs::domain::component_issue::{
    request_component_issue, ComponentIssueItemInput, RequestComponentIssueRequest,
};
use production_rs::domain::fg_receipt::{request_fg_receipt, RequestFgReceiptRequest};
use production_rs::domain::work_orders::{CreateWorkOrderRequest, WorkOrderRepo};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://production_user:production_pass@localhost:5461/production_db".to_string()
    });
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(&url)
        .await
        .expect("Failed to connect to production test DB");
    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run production migrations");
    pool
}

fn unique_tenant() -> String {
    format!("prod-audit-{}", Uuid::new_v4().simple())
}

fn wo_req(tenant: &str) -> CreateWorkOrderRequest {
    CreateWorkOrderRequest {
        tenant_id: tenant.to_string(),
        order_number: format!("WO-{}", &Uuid::new_v4().to_string()[..8]),
        item_id: Uuid::new_v4(),
        bom_revision_id: Uuid::new_v4(),
        routing_template_id: None,
        planned_quantity: 5,
        planned_start: None,
        planned_end: None,
        correlation_id: None,
    }
}

/// Count audit_events rows for a given entity_id + action.
async fn count_audit_events(pool: &sqlx::PgPool, entity_id: &str, action: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM audit_events WHERE entity_id = $1 AND action = $2",
    )
    .bind(entity_id)
    .bind(action)
    .fetch_one(pool)
    .await
    .expect("count audit_events")
}

/// Fetch mutation_class for a given entity_id + action.
async fn fetch_mutation_class(pool: &sqlx::PgPool, entity_id: &str, action: &str) -> String {
    sqlx::query_scalar::<_, String>(
        "SELECT mutation_class::text FROM audit_events WHERE entity_id = $1 AND action = $2 LIMIT 1",
    )
    .bind(entity_id)
    .bind(action)
    .fetch_one(pool)
    .await
    .expect("fetch mutation_class")
}

// ============================================================================
// 1. WorkOrderRepo::create → exactly 1 CREATE audit record
// ============================================================================

#[tokio::test]
#[serial]
async fn audit_oracle_create_work_order() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let wo = WorkOrderRepo::create(&pool, &wo_req(&tenant), &corr, None)
        .await
        .expect("create work order");

    let entity_id = wo.work_order_id.to_string();

    let count = count_audit_events(&pool, &entity_id, "CreateWorkOrder").await;
    assert_eq!(
        count, 1,
        "Expected exactly 1 audit record for CreateWorkOrder"
    );

    let mc = fetch_mutation_class(&pool, &entity_id, "CreateWorkOrder").await;
    assert_eq!(mc, "CREATE", "mutation_class should be CREATE");

    let actor_id: Option<String> = sqlx::query_scalar(
        "SELECT actor_id::text FROM audit_events WHERE entity_id = $1 AND action = $2 LIMIT 1",
    )
    .bind(&entity_id)
    .bind("CreateWorkOrder")
    .fetch_one(&pool)
    .await
    .expect("fetch actor_id");
    assert_eq!(
        actor_id.unwrap_or_default(),
        "00000000-0000-0000-0000-000000000000",
        "actor_id should be nil UUID for system writes"
    );
}

// ============================================================================
// 2. WorkOrderRepo::release → exactly 1 STATE_TRANSITION audit record
// ============================================================================

#[tokio::test]
#[serial]
async fn audit_oracle_release_work_order() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let wo = WorkOrderRepo::create(&pool, &wo_req(&tenant), &corr, None)
        .await
        .expect("create work order");

    WorkOrderRepo::release(&pool, wo.work_order_id, &tenant, &corr, None)
        .await
        .expect("release work order");

    let entity_id = wo.work_order_id.to_string();

    let count = count_audit_events(&pool, &entity_id, "ReleaseWorkOrder").await;
    assert_eq!(
        count, 1,
        "Expected exactly 1 audit record for ReleaseWorkOrder"
    );

    let mc = fetch_mutation_class(&pool, &entity_id, "ReleaseWorkOrder").await;
    assert_eq!(
        mc, "STATE_TRANSITION",
        "mutation_class should be STATE_TRANSITION"
    );
}

// ============================================================================
// 3. WorkOrderRepo::close → exactly 1 STATE_TRANSITION audit record
// ============================================================================

#[tokio::test]
#[serial]
async fn audit_oracle_close_work_order() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let wo = WorkOrderRepo::create(&pool, &wo_req(&tenant), &corr, None)
        .await
        .expect("create work order");

    WorkOrderRepo::release(&pool, wo.work_order_id, &tenant, &corr, None)
        .await
        .expect("release work order");

    WorkOrderRepo::close(&pool, wo.work_order_id, &tenant, &corr, None)
        .await
        .expect("close work order");

    let entity_id = wo.work_order_id.to_string();

    let count = count_audit_events(&pool, &entity_id, "CloseWorkOrder").await;
    assert_eq!(
        count, 1,
        "Expected exactly 1 audit record for CloseWorkOrder"
    );

    let mc = fetch_mutation_class(&pool, &entity_id, "CloseWorkOrder").await;
    assert_eq!(
        mc, "STATE_TRANSITION",
        "mutation_class should be STATE_TRANSITION"
    );
}

// ============================================================================
// 4. request_component_issue → exactly 1 CREATE audit record
// ============================================================================

#[tokio::test]
#[serial]
async fn audit_oracle_component_issue() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    // Need a released WO
    let wo = WorkOrderRepo::create(&pool, &wo_req(&tenant), &corr, None)
        .await
        .expect("create work order");
    WorkOrderRepo::release(&pool, wo.work_order_id, &tenant, &corr, None)
        .await
        .expect("release work order");

    let req = RequestComponentIssueRequest {
        tenant_id: tenant.clone(),
        items: vec![ComponentIssueItemInput {
            item_id: Uuid::new_v4(),
            warehouse_id: Uuid::new_v4(),
            quantity: 3,
            currency: "USD".to_string(),
        }],
        correlation_id: None,
        causation_id: None,
        idempotency_key: None,
    };

    request_component_issue(&pool, wo.work_order_id, &req)
        .await
        .expect("request_component_issue");

    let entity_id = wo.work_order_id.to_string();

    let count = count_audit_events(&pool, &entity_id, "RequestComponentIssue").await;
    assert_eq!(
        count, 1,
        "Expected exactly 1 audit record for RequestComponentIssue"
    );

    let mc = fetch_mutation_class(&pool, &entity_id, "RequestComponentIssue").await;
    assert_eq!(mc, "CREATE", "mutation_class should be CREATE");
}

// ============================================================================
// 5. request_fg_receipt → exactly 1 CREATE audit record
// ============================================================================

#[tokio::test]
#[serial]
async fn audit_oracle_fg_receipt() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    // Need a released WO
    let wo = WorkOrderRepo::create(&pool, &wo_req(&tenant), &corr, None)
        .await
        .expect("create work order");
    WorkOrderRepo::release(&pool, wo.work_order_id, &tenant, &corr, None)
        .await
        .expect("release work order");

    let req = RequestFgReceiptRequest {
        tenant_id: tenant.clone(),
        item_id: Uuid::new_v4(),
        warehouse_id: Uuid::new_v4(),
        quantity: 5,
        currency: "USD".to_string(),
        correlation_id: None,
        causation_id: None,
        idempotency_key: None,
    };

    request_fg_receipt(&pool, wo.work_order_id, &req)
        .await
        .expect("request_fg_receipt");

    let entity_id = wo.work_order_id.to_string();

    let count = count_audit_events(&pool, &entity_id, "RequestFgReceipt").await;
    assert_eq!(
        count, 1,
        "Expected exactly 1 audit record for RequestFgReceipt"
    );

    let mc = fetch_mutation_class(&pool, &entity_id, "RequestFgReceipt").await;
    assert_eq!(mc, "CREATE", "mutation_class should be CREATE");
}
