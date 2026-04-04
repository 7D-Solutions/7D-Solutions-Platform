use quality_inspection_rs::consumers::production_event_bridge::{
    process_fg_receipt_requested, process_operation_completed, FgReceiptRequestedPayload,
    OperationCompletedPayload,
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

fn make_op_completed(tenant_id: &str) -> OperationCompletedPayload {
    OperationCompletedPayload {
        operation_id: Uuid::new_v4(),
        work_order_id: Uuid::new_v4(),
        tenant_id: tenant_id.to_string(),
        operation_name: "CNC Machining".to_string(),
        sequence_number: 10,
    }
}

fn make_fg_receipt(tenant_id: &str) -> FgReceiptRequestedPayload {
    FgReceiptRequestedPayload {
        work_order_id: Uuid::new_v4(),
        tenant_id: tenant_id.to_string(),
        order_number: "WO-2026-001".to_string(),
        item_id: Uuid::new_v4(),
        warehouse_id: Uuid::new_v4(),
        quantity: 50,
        currency: "USD".to_string(),
    }
}

// ============================================================================
// In-process: auto-create from operation_completed
// ============================================================================

#[tokio::test]
#[serial]
async fn auto_creates_in_process_inspection_from_operation_completed() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let event_id = Uuid::new_v4();
    let corr = Uuid::new_v4().to_string();
    let payload = make_op_completed(&tenant);
    let wo_id = payload.work_order_id;
    let op_id = payload.operation_id;

    let result = process_operation_completed(&pool, event_id, &tenant, &payload, &corr, None)
        .await
        .expect("process_operation_completed");

    assert!(result.is_some(), "Should create an in-process inspection");
    let inspection_id = result.unwrap();

    let inspection = service::get_inspection(&pool, &tenant, inspection_id)
        .await
        .expect("get_inspection");

    assert_eq!(inspection.tenant_id, tenant);
    assert_eq!(inspection.inspection_type, "in_process");
    assert_eq!(inspection.result, "pending");
    assert_eq!(inspection.wo_id, Some(wo_id));
    assert_eq!(inspection.op_instance_id, Some(op_id));
    assert!(inspection
        .notes
        .as_deref()
        .unwrap()
        .contains("Auto-created from operation completed"));
}

// ============================================================================
// In-process: dedup by event_id
// ============================================================================

#[tokio::test]
#[serial]
async fn in_process_dedup_by_event_id() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let event_id = Uuid::new_v4();
    let corr = Uuid::new_v4().to_string();
    let payload = make_op_completed(&tenant);
    let wo_id = payload.work_order_id;

    // First call — creates inspection
    let first = process_operation_completed(&pool, event_id, &tenant, &payload, &corr, None)
        .await
        .expect("first call");
    assert!(first.is_some());

    // Second call with SAME event_id — should skip
    let second = process_operation_completed(&pool, event_id, &tenant, &payload, &corr, None)
        .await
        .expect("second call");
    assert!(second.is_none(), "Duplicate event should be skipped");

    // Verify only one in-process inspection for this WO
    let inspections = service::list_inspections_by_wo(&pool, &tenant, wo_id, Some("in_process"))
        .await
        .expect("list_by_wo");
    assert_eq!(inspections.len(), 1, "Should have exactly one inspection");
}

// ============================================================================
// In-process: semantic dedup by (wo_id, op_instance_id)
// ============================================================================

#[tokio::test]
#[serial]
async fn in_process_semantic_dedup_by_wo_and_op() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();
    let payload = make_op_completed(&tenant);
    let wo_id = payload.work_order_id;

    // First call with event A
    let first = process_operation_completed(
        &pool,
        Uuid::new_v4(),
        &tenant,
        &payload,
        &corr,
        None,
    )
    .await
    .expect("first call");
    assert!(first.is_some());

    // Second call with different event_id but SAME wo_id + op_id
    let second = process_operation_completed(
        &pool,
        Uuid::new_v4(),
        &tenant,
        &payload,
        &corr,
        None,
    )
    .await
    .expect("second call");
    assert!(
        second.is_none(),
        "Should skip — inspection already exists for this WO+op"
    );

    let inspections = service::list_inspections_by_wo(&pool, &tenant, wo_id, Some("in_process"))
        .await
        .expect("list_by_wo");
    assert_eq!(inspections.len(), 1, "Should have exactly one inspection");
}

// ============================================================================
// In-process: different ops on same WO create separate inspections
// ============================================================================

#[tokio::test]
#[serial]
async fn different_ops_create_separate_inspections() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();
    let wo_id = Uuid::new_v4();

    let payload1 = OperationCompletedPayload {
        operation_id: Uuid::new_v4(),
        work_order_id: wo_id,
        tenant_id: tenant.clone(),
        operation_name: "Op 10 - Mill".to_string(),
        sequence_number: 10,
    };

    let payload2 = OperationCompletedPayload {
        operation_id: Uuid::new_v4(),
        work_order_id: wo_id,
        tenant_id: tenant.clone(),
        operation_name: "Op 20 - Drill".to_string(),
        sequence_number: 20,
    };

    let first = process_operation_completed(
        &pool,
        Uuid::new_v4(),
        &tenant,
        &payload1,
        &corr,
        None,
    )
    .await
    .expect("first op");
    assert!(first.is_some());

    let second = process_operation_completed(
        &pool,
        Uuid::new_v4(),
        &tenant,
        &payload2,
        &corr,
        None,
    )
    .await
    .expect("second op");
    assert!(second.is_some());

    let inspections = service::list_inspections_by_wo(&pool, &tenant, wo_id, Some("in_process"))
        .await
        .expect("list_by_wo");
    assert_eq!(
        inspections.len(),
        2,
        "Different ops should create separate inspections"
    );
}

// ============================================================================
// Final: auto-create from fg_receipt.requested
// ============================================================================

#[tokio::test]
#[serial]
async fn auto_creates_final_inspection_from_fg_receipt() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let event_id = Uuid::new_v4();
    let corr = Uuid::new_v4().to_string();
    let payload = make_fg_receipt(&tenant);
    let wo_id = payload.work_order_id;
    let item_id = payload.item_id;

    let result = process_fg_receipt_requested(&pool, event_id, &tenant, &payload, &corr, None)
        .await
        .expect("process_fg_receipt_requested");

    assert!(result.is_some(), "Should create a final inspection");
    let inspection_id = result.unwrap();

    let inspection = service::get_inspection(&pool, &tenant, inspection_id)
        .await
        .expect("get_inspection");

    assert_eq!(inspection.tenant_id, tenant);
    assert_eq!(inspection.inspection_type, "final");
    assert_eq!(inspection.result, "pending");
    assert_eq!(inspection.wo_id, Some(wo_id));
    assert_eq!(inspection.part_id, Some(item_id));
    assert!(inspection
        .notes
        .as_deref()
        .unwrap()
        .contains("Auto-created from FG receipt request"));
}

// ============================================================================
// Final: dedup by event_id
// ============================================================================

#[tokio::test]
#[serial]
async fn final_dedup_by_event_id() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let event_id = Uuid::new_v4();
    let corr = Uuid::new_v4().to_string();
    let payload = make_fg_receipt(&tenant);
    let wo_id = payload.work_order_id;

    let first = process_fg_receipt_requested(&pool, event_id, &tenant, &payload, &corr, None)
        .await
        .expect("first call");
    assert!(first.is_some());

    let second = process_fg_receipt_requested(&pool, event_id, &tenant, &payload, &corr, None)
        .await
        .expect("second call");
    assert!(second.is_none(), "Duplicate event should be skipped");

    let inspections = service::list_inspections_by_wo(&pool, &tenant, wo_id, Some("final"))
        .await
        .expect("list_by_wo");
    assert_eq!(inspections.len(), 1, "Should have exactly one final inspection");
}

// ============================================================================
// Processed events recorded in dedup table
// ============================================================================

#[tokio::test]
#[serial]
async fn records_processed_events_in_dedup_table() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let op_event_id = Uuid::new_v4();
    let op_payload = make_op_completed(&tenant);
    process_operation_completed(&pool, op_event_id, &tenant, &op_payload, &corr, None)
        .await
        .expect("process op");

    let fg_event_id = Uuid::new_v4();
    let fg_payload = make_fg_receipt(&tenant);
    process_fg_receipt_requested(&pool, fg_event_id, &tenant, &fg_payload, &corr, None)
        .await
        .expect("process fg");

    // Verify both events recorded
    let op_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM quality_inspection_processed_events WHERE event_id = $1 AND processor = 'production_event_bridge_in_process'",
    )
    .bind(op_event_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(op_count.0, 1);

    let fg_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM quality_inspection_processed_events WHERE event_id = $1 AND processor = 'production_event_bridge_final'",
    )
    .bind(fg_event_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(fg_count.0, 1);
}

// ============================================================================
// Outbox events emitted for auto-created inspections
// ============================================================================

#[tokio::test]
#[serial]
async fn outbox_events_emitted_for_auto_created_inspections() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    // Create in-process inspection via bridge
    let op_payload = make_op_completed(&tenant);
    process_operation_completed(&pool, Uuid::new_v4(), &tenant, &op_payload, &corr, None)
        .await
        .expect("process op");

    // Create final inspection via bridge
    let fg_payload = make_fg_receipt(&tenant);
    process_fg_receipt_requested(&pool, Uuid::new_v4(), &tenant, &fg_payload, &corr, None)
        .await
        .expect("process fg");

    // Verify outbox has both events
    let event_types: Vec<(String,)> = sqlx::query_as(
        "SELECT event_type FROM quality_inspection_outbox WHERE tenant_id = $1 ORDER BY created_at",
    )
    .bind(&tenant)
    .fetch_all(&pool)
    .await
    .unwrap();

    let types: Vec<&str> = event_types.iter().map(|r| r.0.as_str()).collect();
    assert_eq!(types.len(), 2);
    assert!(types.contains(&"quality_inspection.inspection_recorded"));

    // Verify in-process event payload
    let payloads: Vec<(serde_json::Value,)> = sqlx::query_as(
        "SELECT payload FROM quality_inspection_outbox WHERE tenant_id = $1 ORDER BY created_at",
    )
    .bind(&tenant)
    .fetch_all(&pool)
    .await
    .unwrap();

    let in_process_payload = &payloads[0].0;
    assert_eq!(
        in_process_payload["payload"]["inspection_type"],
        "in_process"
    );
    assert!(in_process_payload["payload"]["wo_id"].is_string());
    assert!(in_process_payload["payload"]["op_instance_id"].is_string());

    let final_payload = &payloads[1].0;
    assert_eq!(final_payload["payload"]["inspection_type"], "final");
    assert!(final_payload["payload"]["wo_id"].is_string());
}

// ============================================================================
// Queryable: in-process and final inspections by WO
// ============================================================================

#[tokio::test]
#[serial]
async fn bridge_created_inspections_queryable_by_wo() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();
    let wo_id = Uuid::new_v4();

    // In-process via bridge
    let op_payload = OperationCompletedPayload {
        operation_id: Uuid::new_v4(),
        work_order_id: wo_id,
        tenant_id: tenant.clone(),
        operation_name: "Op 10".to_string(),
        sequence_number: 10,
    };
    process_operation_completed(&pool, Uuid::new_v4(), &tenant, &op_payload, &corr, None)
        .await
        .expect("process op");

    // Final via bridge
    let fg_payload = FgReceiptRequestedPayload {
        work_order_id: wo_id,
        tenant_id: tenant.clone(),
        order_number: "WO-001".to_string(),
        item_id: Uuid::new_v4(),
        warehouse_id: Uuid::new_v4(),
        quantity: 10,
        currency: "USD".to_string(),
    };
    process_fg_receipt_requested(&pool, Uuid::new_v4(), &tenant, &fg_payload, &corr, None)
        .await
        .expect("process fg");

    // Query all inspections for this WO
    let all = service::list_inspections_by_wo(&pool, &tenant, wo_id, None)
        .await
        .unwrap();
    assert_eq!(all.len(), 2);

    let in_process = service::list_inspections_by_wo(&pool, &tenant, wo_id, Some("in_process"))
        .await
        .unwrap();
    assert_eq!(in_process.len(), 1);

    let finals = service::list_inspections_by_wo(&pool, &tenant, wo_id, Some("final"))
        .await
        .unwrap();
    assert_eq!(finals.len(), 1);
}
