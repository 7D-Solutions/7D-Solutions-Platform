use chrono::Utc;
use quality_inspection_rs::consumers::receipt_event_bridge::{
    process_item_received, ItemReceivedPayload,
};
use quality_inspection_rs::domain::service;
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://quality_inspection_user:quality_inspection_pass@localhost:5459/quality_inspection_db?sslmode=require".to_string()
    });

    let pool = PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(&url)
        .await
        .expect("Failed to connect to quality-inspection test DB");

    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run quality-inspection migrations");

    pool
}

fn unique_tenant() -> String {
    Uuid::new_v4().to_string()
}

fn make_purchase_receipt(tenant_id: &str) -> ItemReceivedPayload {
    ItemReceivedPayload {
        receipt_line_id: Uuid::new_v4(),
        tenant_id: tenant_id.to_string(),
        item_id: Uuid::new_v4(),
        sku: "BOLT-M8-50".to_string(),
        warehouse_id: Uuid::new_v4(),
        quantity: 100,
        unit_cost_minor: 250,
        currency: "USD".to_string(),
        source_type: "purchase".to_string(),
        purchase_order_id: Some(Uuid::new_v4()),
        received_at: Utc::now(),
    }
}

// ============================================================================
// Auto-create receiving inspection from purchase receipt
// ============================================================================

#[tokio::test]
#[serial]
async fn auto_creates_inspection_from_purchase_receipt() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let event_id = Uuid::new_v4();
    let corr = Uuid::new_v4().to_string();
    let payload = make_purchase_receipt(&tenant);
    let receipt_line_id = payload.receipt_line_id;

    let result = process_item_received(&pool, event_id, &tenant, &payload, &corr, None)
        .await
        .expect("process_item_received");

    assert!(result.is_some(), "Should create an inspection");
    let inspection_id = result.unwrap();

    // Verify the inspection exists and has correct fields
    let inspection = service::get_inspection(&pool, &tenant, inspection_id)
        .await
        .expect("get_inspection");

    assert_eq!(inspection.tenant_id, tenant);
    assert_eq!(inspection.inspection_type, "receiving");
    assert_eq!(inspection.result, "pending");
    assert_eq!(inspection.receipt_id, Some(receipt_line_id));
    assert_eq!(inspection.part_id, Some(payload.item_id));
    assert!(inspection
        .notes
        .as_deref()
        .unwrap()
        .contains("Auto-created"));

    // Verify the inspections are queryable by receipt
    let by_receipt = service::list_inspections_by_receipt(&pool, &tenant, receipt_line_id)
        .await
        .expect("list_by_receipt");
    assert_eq!(by_receipt.len(), 1);
    assert_eq!(by_receipt[0].id, inspection_id);
}

// ============================================================================
// Auto-create from return receipt
// ============================================================================

#[tokio::test]
#[serial]
async fn auto_creates_inspection_from_return_receipt() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let event_id = Uuid::new_v4();
    let corr = Uuid::new_v4().to_string();
    let mut payload = make_purchase_receipt(&tenant);
    payload.source_type = "return".to_string();

    let result = process_item_received(&pool, event_id, &tenant, &payload, &corr, None)
        .await
        .expect("process_item_received");

    assert!(result.is_some(), "Should create an inspection for returns");
}

// ============================================================================
// Skips production receipts
// ============================================================================

#[tokio::test]
#[serial]
async fn skips_production_receipt() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let event_id = Uuid::new_v4();
    let corr = Uuid::new_v4().to_string();
    let mut payload = make_purchase_receipt(&tenant);
    payload.source_type = "production".to_string();

    let result = process_item_received(&pool, event_id, &tenant, &payload, &corr, None)
        .await
        .expect("process_item_received");

    assert!(result.is_none(), "Should NOT create inspection for production receipts");
}

// ============================================================================
// Duplicate events do not create duplicate inspections
// ============================================================================

#[tokio::test]
#[serial]
async fn duplicate_event_does_not_create_duplicate_inspection() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let event_id = Uuid::new_v4();
    let corr = Uuid::new_v4().to_string();
    let payload = make_purchase_receipt(&tenant);
    let receipt_line_id = payload.receipt_line_id;

    // First call — creates inspection
    let first = process_item_received(&pool, event_id, &tenant, &payload, &corr, None)
        .await
        .expect("first call");
    assert!(first.is_some());

    // Second call with SAME event_id — should skip
    let second = process_item_received(&pool, event_id, &tenant, &payload, &corr, None)
        .await
        .expect("second call");
    assert!(second.is_none(), "Duplicate event should be skipped");

    // Verify only one inspection exists for this receipt
    let inspections = service::list_inspections_by_receipt(&pool, &tenant, receipt_line_id)
        .await
        .expect("list_by_receipt");
    assert_eq!(inspections.len(), 1, "Should have exactly one inspection");
}

// ============================================================================
// Processed event recorded in dedup table
// ============================================================================

#[tokio::test]
#[serial]
async fn records_processed_event_in_dedup_table() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let event_id = Uuid::new_v4();
    let corr = Uuid::new_v4().to_string();
    let payload = make_purchase_receipt(&tenant);

    process_item_received(&pool, event_id, &tenant, &payload, &corr, None)
        .await
        .expect("process_item_received");

    // Verify the event is recorded in the processed_events table
    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM quality_inspection_processed_events WHERE event_id = $1 AND processor = 'receipt_event_bridge'",
    )
    .bind(event_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(count.0, 1, "Event should be recorded in processed_events");
}

// ============================================================================
// Outbox event emitted for auto-created inspection
// ============================================================================

#[tokio::test]
#[serial]
async fn outbox_event_emitted_for_auto_created_inspection() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let event_id = Uuid::new_v4();
    let corr = Uuid::new_v4().to_string();
    let payload = make_purchase_receipt(&tenant);

    process_item_received(&pool, event_id, &tenant, &payload, &corr, None)
        .await
        .expect("process_item_received");

    // Verify outbox has an inspection_recorded event
    let event_types: Vec<(String,)> = sqlx::query_as(
        "SELECT event_type FROM quality_inspection_outbox WHERE tenant_id = $1",
    )
    .bind(&tenant)
    .fetch_all(&pool)
    .await
    .unwrap();

    let types: Vec<&str> = event_types.iter().map(|r| r.0.as_str()).collect();
    assert!(
        types.contains(&"quality_inspection.inspection_recorded"),
        "Should emit inspection_recorded event to outbox"
    );
}
