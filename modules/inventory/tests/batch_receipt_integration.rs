//! Integration tests for the batch stock receipt flow.
//!
//! Tests run against a real PostgreSQL database.
//! Set DATABASE_URL to the inventory database connection string.

use axum::{
    body::Body,
    http::{Request, StatusCode},
    routing::post,
    Json, Router,
};
use http_body_util::BodyExt; // for `collect`
use inventory_rs::{
    db::resolver::resolve_pool,
    domain::{
        items::{CreateItemRequest, ItemRepo, TrackingMode},
        receipt_service::ReceiptRequest,
    },
    http::batch_receipts::{BatchReceiptItemResult, BatchReceiptRequest, BatchReceiptResponse},
    AppState, Config,
};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use tower::ServiceExt; // for `call`, `ready`
use uuid::Uuid;

// ============================================================================
// Test DB and App helpers
// ============================================================================

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://inventory_user:inventory_pass@localhost:5442/inventory_db?sslmode=disable"
            .to_string()
    });

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to inventory test DB");

    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run inventory migrations");

    pool
}

fn create_item_req(tenant_id: &str, sku: &str) -> CreateItemRequest {
    CreateItemRequest {
        tenant_id: tenant_id.to_string(),
        sku: sku.to_string(),
        name: "Test Batch Widget".to_string(),
        description: None,
        inventory_account_ref: "1200".to_string(),
        cogs_account_ref: "5000".to_string(),
        variance_account_ref: "5010".to_string(),
        uom: None,
        tracking_mode: TrackingMode::None,
        make_buy: None,
    }
}

fn receipt_req_for_batch(tenant_id: &str, item_id: Uuid, idem_key: &str) -> ReceiptRequest {
    ReceiptRequest {
        tenant_id: tenant_id.to_string(),
        item_id,
        warehouse_id: Uuid::new_v4(),
        quantity: 10,
        unit_cost_minor: 100, // $1.00
        currency: "usd".to_string(),
        source_type: "purchase".to_string(),
        purchase_order_id: None,
        idempotency_key: idem_key.to_string(),
        correlation_id: None,
        causation_id: None,
        lot_code: None,
        serial_codes: None,
        location_id: None,
        uom_id: None,
    }
}

async fn create_app(pool: sqlx::PgPool) -> Router {
    let config = Config {
        database_url: "dummy".to_string(), // Not used for direct pool connection
        host: "0.0.0.0".to_string(),
        port: 8092,
        env: "test".to_string(),
        cors_origins: vec!["*".to_string()],
    };
    let metrics = Arc::new(inventory_rs::metrics::InventoryMetrics::new().unwrap());
    let app_state = Arc::new(AppState { pool, metrics });

    Router::new()
        .route(
            "/api/inventory/batch-receipts",
            post(inventory_rs::http::batch_receipts::post_batch_receipts),
        )
        .with_state(app_state)
}

// ============================================================================
// Happy-path: multiple ledger rows + layers + outbox created atomically
// ============================================================================

#[tokio::test]
#[serial]
async fn batch_receipt_processes_multiple_items_successfully() {
    let pool = setup_db().await;
    let app = create_app(pool.clone()).await;
    let tenant_id = format!("test-{}", Uuid::new_v4());

    // Create items
    let item1 = ItemRepo::create(&pool, &create_item_req(&tenant_id, "SKU-BR-001"))
        .await
        .expect("create item 1");
    let item2 = ItemRepo::create(&pool, &create_item_req(&tenant_id, "SKU-BR-002"))
        .await
        .expect("create item 2");

    let req1 = receipt_req_for_batch(&tenant_id, item1.id, &format!("idem-{}", Uuid::new_v4()));
    let req2 = receipt_req_for_batch(&tenant_id, item2.id, &format!("idem-{}", Uuid::new_v4()));

    let batch_req = BatchReceiptRequest {
        receipts: vec![req1.clone(), req2.clone()],
    };

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/inventory/batch-receipts")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&batch_req).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let batch_res: BatchReceiptResponse = serde_json::from_bytes(&body).unwrap();

    assert_eq!(batch_res.results.len(), 2);

    // Verify item1 result
    match &batch_res.results[0] {
        BatchReceiptItemResult::Success(res) => {
            assert_eq!(res.item_id, item1.id);
            assert_eq!(res.quantity, req1.quantity);
        }
        BatchReceiptItemResult::Error { .. } => panic!("Expected success for item1"),
    }

    // Verify item2 result
    match &batch_res.results[1] {
        BatchReceiptItemResult::Success(res) => {
            assert_eq!(res.item_id, item2.id);
            assert_eq!(res.quantity, req2.quantity);
        }
        BatchReceiptItemResult::Error { .. } => panic!("Expected success for item2"),
    }

    // Verify DB state for item1
    let ledger_count1: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inventory_ledger WHERE tenant_id = $1 AND item_id = $2",
    )
    .bind(&tenant_id)
    .bind(item1.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(ledger_count1, 1);

    // Verify DB state for item2
    let ledger_count2: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inventory_ledger WHERE tenant_id = $1 AND item_id = $2",
    )
    .bind(&tenant_id)
    .bind(item2.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(ledger_count2, 1);

    // Verify total ledger entries
    let total_ledger_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM inventory_ledger WHERE tenant_id = $1")
            .bind(&tenant_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(total_ledger_count, 2);

    // Cleanup
    sqlx::query("DELETE FROM inv_outbox WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM inv_idempotency_keys WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM inventory_layers WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM inventory_ledger WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM item_on_hand_by_status WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM item_on_hand WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM items WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
}

// ============================================================================
// Partial Failure: one valid, one invalid item
// ============================================================================

#[tokio::test]
#[serial]
async fn batch_receipt_handles_partial_failure() {
    let pool = setup_db().await;
    let app = create_app(pool.clone()).await;
    let tenant_id = format!("test-{}", Uuid::new_v4());

    // Create one active item and one inactive item
    let item_active = ItemRepo::create(&pool, &create_item_req(&tenant_id, "SKU-VALID-001"))
        .await
        .expect("create active item");
    let item_inactive = ItemRepo::create(&pool, &create_item_req(&tenant_id, "SKU-INVALID-001"))
        .await
        .expect("create inactive item");

    ItemRepo::deactivate(&pool, item_inactive.id, &tenant_id)
        .await
        .expect("deactivate item");

    let req_valid = receipt_req_for_batch(
        &tenant_id,
        item_active.id,
        &format!("idem-{}", Uuid::new_v4()),
    );
    let req_invalid = receipt_req_for_batch(
        &tenant_id,
        item_inactive.id,
        &format!("idem-{}", Uuid::new_v4()),
    );

    let batch_req = BatchReceiptRequest {
        receipts: vec![req_valid.clone(), req_invalid.clone()],
    };

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/inventory/batch-receipts")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&batch_req).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let batch_res: BatchReceiptResponse = serde_json::from_bytes(&body).unwrap();

    assert_eq!(batch_res.results.len(), 2);

    // Verify valid item result
    match &batch_res.results[0] {
        BatchReceiptItemResult::Success(res) => {
            assert_eq!(res.item_id, item_active.id);
        }
        BatchReceiptItemResult::Error { .. } => panic!("Expected success for valid item"),
    }

    // Verify invalid item result
    match &batch_res.results[1] {
        BatchReceiptItemResult::Error {
            item_id,
            error_message,
        } => {
            assert_eq!(*item_id, item_inactive.id);
            assert!(
                error_message.contains("Guard failed: Item is not active"),
                "Error message was: {}",
                error_message
            );
        }
        BatchReceiptItemResult::Success(_) => panic!("Expected error for invalid item"),
    }

    // Verify DB state: only valid item should have created ledger entry
    let ledger_count_valid: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inventory_ledger WHERE tenant_id = $1 AND item_id = $2",
    )
    .bind(&tenant_id)
    .bind(item_active.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(ledger_count_valid, 1);

    let ledger_count_invalid: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inventory_ledger WHERE tenant_id = $1 AND item_id = $2",
    )
    .bind(&tenant_id)
    .bind(item_inactive.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(ledger_count_invalid, 0);

    // Cleanup
    sqlx::query("DELETE FROM inv_outbox WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM inv_idempotency_keys WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM inventory_layers WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM inventory_ledger WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM item_on_hand_by_status WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM item_on_hand WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM items WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
}

// ============================================================================
// Idempotency: replaying a batch request
// ============================================================================

#[tokio::test]
#[serial]
async fn batch_receipt_idempotency_replay_returns_stored_results() {
    let pool = setup_db().await;
    let app = create_app(pool.clone()).await;
    let tenant_id = format!("test-{}", Uuid::new_v4());

    // Create items
    let item1 = ItemRepo::create(&pool, &create_item_req(&tenant_id, "SKU-IDEM-BATCH-001"))
        .await
        .expect("create item 1");
    let item2 = ItemRepo::create(&pool, &create_item_req(&tenant_id, "SKU-IDEM-BATCH-002"))
        .await
        .expect("create item 2");

    let idem_key1 = format!("idem-{}", Uuid::new_v4());
    let idem_key2 = format!("idem-{}", Uuid::new_v4());

    let req1 = receipt_req_for_batch(&tenant_id, item1.id, &idem_key1);
    let req2 = receipt_req_for_batch(&tenant_id, item2.id, &idem_key2);

    let batch_req = BatchReceiptRequest {
        receipts: vec![req1.clone(), req2.clone()],
    };

    // First call
    let response1 = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/inventory/batch-receipts")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&batch_req).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response1.status(), StatusCode::OK);
    let body1 = response1.into_body().collect().await.unwrap().to_bytes();
    let batch_res1: BatchReceiptResponse = serde_json::from_bytes(&body1).unwrap();

    // Second call with same keys and bodies
    let response2 = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/inventory/batch-receipts")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&batch_req).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response2.status(), StatusCode::OK);
    let body2 = response2.into_body().collect().await.unwrap().to_bytes();
    let batch_res2: BatchReceiptResponse = serde_json::from_bytes(&body2).unwrap();

    assert_eq!(batch_res1.results.len(), 2);
    assert_eq!(batch_res2.results.len(), 2);

    // For idempotency, the individual ReceiptResult's event_id, layer_id etc. should be the same
    match (&batch_res1.results[0], &batch_res2.results[0]) {
        (BatchReceiptItemResult::Success(res1), BatchReceiptItemResult::Success(res2)) => {
            assert_eq!(res1.event_id, res2.event_id);
            assert_eq!(res1.receipt_line_id, res2.receipt_line_id);
        }
        _ => panic!("Expected success results"),
    }
    match (&batch_res1.results[1], &batch_res2.results[1]) {
        (BatchReceiptItemResult::Success(res1), BatchReceiptItemResult::Success(res2)) => {
            assert_eq!(res1.event_id, res2.event_id);
            assert_eq!(res1.receipt_line_id, res2.receipt_line_id);
        }
        _ => panic!("Expected success results"),
    }

    // Verify DB state: total ledger entries should still be 2 (no duplicates)
    let total_ledger_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM inventory_ledger WHERE tenant_id = $1")
            .bind(&tenant_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(total_ledger_count, 2);

    // Verify that idempotency keys were recorded for both
    let idem_keys_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inv_idempotency_keys WHERE tenant_id = $1 AND (idempotency_key = $2 OR idempotency_key = $3)",
    )
    .bind(&tenant_id)
    .bind(&idem_key1)
    .bind(&idem_key2)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(idem_keys_count, 2);

    // Cleanup
    sqlx::query("DELETE FROM inv_outbox WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM inv_idempotency_keys WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM inventory_layers WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM inventory_ledger WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM item_on_hand_by_status WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM item_on_hand WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM items WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
}
