use production_rs::domain::fg_receipt::{
    request_fg_receipt, FgReceiptError, RequestFgReceiptRequest,
};
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
        .max_connections(2)
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
    format!("test-fgr-{}", Uuid::new_v4())
}

async fn create_released_wo(pool: &sqlx::PgPool, tenant: &str) -> (Uuid, Uuid) {
    let corr = Uuid::new_v4().to_string();
    let item_id = Uuid::new_v4();
    let wo = WorkOrderRepo::create(
        pool,
        &CreateWorkOrderRequest {
            tenant_id: tenant.to_string(),
            order_number: format!("WO-{}", &Uuid::new_v4().to_string()[..8]),
            item_id,
            bom_revision_id: Uuid::new_v4(),
            routing_template_id: None,
            planned_quantity: 10,
            planned_start: None,
            planned_end: None,
            correlation_id: None,
        },
        &corr,
        None,
    )
    .await
    .expect("create work order");

    WorkOrderRepo::release(pool, wo.work_order_id, tenant, &corr, None)
        .await
        .expect("release work order");

    (wo.work_order_id, item_id)
}

// ============================================================================
// Happy path: request FG receipt → outbox event
// ============================================================================

#[tokio::test]
#[serial]
async fn fg_receipt_request_creates_outbox_event() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let (wo_id, item_id) = create_released_wo(&pool, &tenant).await;
    let warehouse_id = Uuid::new_v4();

    let req = RequestFgReceiptRequest {
        tenant_id: tenant.clone(),
        item_id,
        warehouse_id,
        quantity: 5,
        currency: "usd".to_string(),
        correlation_id: Some("fgr-test-corr".to_string()),
        causation_id: None,
        idempotency_key: None,
    };

    request_fg_receipt(&pool, wo_id, &req)
        .await
        .expect("request fg receipt");

    // Verify outbox event
    let events = sqlx::query_as::<_, (String, serde_json::Value)>(
        "SELECT event_type, payload FROM production_outbox WHERE aggregate_id = $1 AND event_type = 'production.fg_receipt.requested'",
    )
    .bind(wo_id.to_string())
    .fetch_all(&pool)
    .await
    .expect("fetch outbox");

    assert!(!events.is_empty(), "outbox should contain the event");

    let (event_type, payload) = &events[0];
    assert_eq!(event_type, "production.fg_receipt.requested");

    let p = &payload["payload"];
    assert_eq!(p["work_order_id"].as_str(), Some(wo_id.to_string().as_str()));
    assert_eq!(p["tenant_id"].as_str(), Some(tenant.as_str()));
    assert_eq!(p["item_id"].as_str(), Some(item_id.to_string().as_str()));
    assert_eq!(p["warehouse_id"].as_str(), Some(warehouse_id.to_string().as_str()));
    assert_eq!(p["quantity"].as_i64(), Some(5));
    assert_eq!(p["currency"].as_str(), Some("usd"));
}

// ============================================================================
// Rejected: draft work order
// ============================================================================

#[tokio::test]
#[serial]
async fn fg_receipt_rejects_draft_wo() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let wo = WorkOrderRepo::create(
        &pool,
        &CreateWorkOrderRequest {
            tenant_id: tenant.clone(),
            order_number: format!("WO-{}", &Uuid::new_v4().to_string()[..8]),
            item_id: Uuid::new_v4(),
            bom_revision_id: Uuid::new_v4(),
            routing_template_id: None,
            planned_quantity: 5,
            planned_start: None,
            planned_end: None,
            correlation_id: None,
        },
        &corr,
        None,
    )
    .await
    .expect("create draft WO");

    let req = RequestFgReceiptRequest {
        tenant_id: tenant.clone(),
        item_id: Uuid::new_v4(),
        warehouse_id: Uuid::new_v4(),
        quantity: 5,
        currency: "usd".to_string(),
        correlation_id: None,
        causation_id: None,
        idempotency_key: None,
    };

    let err = request_fg_receipt(&pool, wo.work_order_id, &req)
        .await
        .expect_err("should reject draft WO");

    assert!(
        matches!(err, FgReceiptError::NotReleased),
        "Expected NotReleased, got: {:?}",
        err
    );
}

// ============================================================================
// Rejected: zero quantity
// ============================================================================

#[tokio::test]
#[serial]
async fn fg_receipt_rejects_zero_quantity() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let (wo_id, _) = create_released_wo(&pool, &tenant).await;

    let req = RequestFgReceiptRequest {
        tenant_id: tenant.clone(),
        item_id: Uuid::new_v4(),
        warehouse_id: Uuid::new_v4(),
        quantity: 0,
        currency: "usd".to_string(),
        correlation_id: None,
        causation_id: None,
        idempotency_key: None,
    };

    let err = request_fg_receipt(&pool, wo_id, &req)
        .await
        .expect_err("should reject zero quantity");

    assert!(
        matches!(err, FgReceiptError::Validation(_)),
        "Expected Validation, got: {:?}",
        err
    );
}

// ============================================================================
// Rejected: missing work order
// ============================================================================

#[tokio::test]
#[serial]
async fn fg_receipt_rejects_missing_wo() {
    let pool = setup_db().await;

    let req = RequestFgReceiptRequest {
        tenant_id: "some-tenant".to_string(),
        item_id: Uuid::new_v4(),
        warehouse_id: Uuid::new_v4(),
        quantity: 5,
        currency: "usd".to_string(),
        correlation_id: None,
        causation_id: None,
        idempotency_key: None,
    };

    let err = request_fg_receipt(&pool, Uuid::new_v4(), &req)
        .await
        .expect_err("should reject missing WO");

    assert!(
        matches!(err, FgReceiptError::NotFound),
        "Expected NotFound, got: {:?}",
        err
    );
}
