//! Integration tests for the batch stock receipt flow.
//!
//! Tests exercise the batch receipt handler logic through the service layer.
//! All tests run against a real PostgreSQL database — no mocks.

use inventory_rs::{
    domain::{
        items::{CreateItemRequest, ItemRepo, TrackingMode},
        receipt_service::{self, ReceiptRequest},
    },
    http::batch_receipts::{BatchReceiptItemResult, BatchReceiptRequest},
};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

// ============================================================================
// Test DB helpers
// ============================================================================

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://inventory_user:inventory_pass@localhost:5442/inventory_db?sslmode=require"
            .to_string()
    });

    let pool = PgPoolOptions::new()
        .max_connections(10)
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

fn receipt_req(tenant_id: &str, item_id: Uuid, idem_key: &str) -> ReceiptRequest {
    ReceiptRequest {
        tenant_id: tenant_id.to_string(),
        item_id,
        warehouse_id: Uuid::new_v4(),
        quantity: 10,
        unit_cost_minor: 100,
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

/// Process a batch of receipt requests (mirrors the HTTP handler logic).
async fn process_batch(
    pool: &sqlx::PgPool,
    batch: &BatchReceiptRequest,
) -> Vec<BatchReceiptItemResult> {
    let mut results = Vec::with_capacity(batch.receipts.len());
    for req in &batch.receipts {
        match receipt_service::process_receipt(pool, req, None).await {
            Ok((result, _is_replay)) => {
                results.push(BatchReceiptItemResult::Success(Box::new(result)));
            }
            Err(e) => {
                results.push(BatchReceiptItemResult::Error {
                    item_id: req.item_id,
                    error_message: e.to_string(),
                });
            }
        }
    }
    results
}

async fn cleanup(pool: &sqlx::PgPool, tenant_id: &str) {
    for table in [
        "inv_outbox",
        "inv_idempotency_keys",
        "inventory_layers",
        "inventory_ledger",
        "item_on_hand_by_status",
        "item_on_hand",
        "items",
    ] {
        sqlx::query(&format!("DELETE FROM {} WHERE tenant_id = $1", table))
            .bind(tenant_id)
            .execute(pool)
            .await
            .ok();
    }
}

// ============================================================================
// Happy path: multiple receipts processed successfully
// ============================================================================

#[tokio::test]
#[serial]
async fn batch_receipt_processes_multiple_items_successfully() {
    let pool = setup_db().await;
    let tenant_id = format!("test-{}", Uuid::new_v4());

    let item1 = ItemRepo::create(&pool, &create_item_req(&tenant_id, "SKU-BR-001"))
        .await
        .expect("create item 1");
    let item2 = ItemRepo::create(&pool, &create_item_req(&tenant_id, "SKU-BR-002"))
        .await
        .expect("create item 2");

    let batch = BatchReceiptRequest {
        receipts: vec![
            receipt_req(&tenant_id, item1.id, &format!("idem-{}", Uuid::new_v4())),
            receipt_req(&tenant_id, item2.id, &format!("idem-{}", Uuid::new_v4())),
        ],
    };

    let results: Vec<BatchReceiptItemResult> = process_batch(&pool, &batch).await;

    assert_eq!(results.len(), 2);
    assert!(matches!(&results[0], BatchReceiptItemResult::Success(r) if r.item_id == item1.id));
    assert!(matches!(&results[1], BatchReceiptItemResult::Success(r) if r.item_id == item2.id));

    // Verify DB state
    let total: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM inventory_ledger WHERE tenant_id = $1")
            .bind(&tenant_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(total, 2);

    cleanup(&pool, &tenant_id).await;
}

// ============================================================================
// Partial failure: one valid, one invalid item
// ============================================================================

#[tokio::test]
#[serial]
async fn batch_receipt_handles_partial_failure() {
    let pool = setup_db().await;
    let tenant_id = format!("test-{}", Uuid::new_v4());

    let active = ItemRepo::create(&pool, &create_item_req(&tenant_id, "SKU-VALID-001"))
        .await
        .expect("create active");
    let inactive = ItemRepo::create(&pool, &create_item_req(&tenant_id, "SKU-INVALID-001"))
        .await
        .expect("create inactive");

    ItemRepo::deactivate(&pool, inactive.id, &tenant_id)
        .await
        .expect("deactivate");

    let batch = BatchReceiptRequest {
        receipts: vec![
            receipt_req(&tenant_id, active.id, &format!("idem-{}", Uuid::new_v4())),
            receipt_req(&tenant_id, inactive.id, &format!("idem-{}", Uuid::new_v4())),
        ],
    };

    let results: Vec<BatchReceiptItemResult> = process_batch(&pool, &batch).await;

    assert_eq!(results.len(), 2);
    assert!(matches!(&results[0], BatchReceiptItemResult::Success(_)));
    assert!(matches!(&results[1], BatchReceiptItemResult::Error { .. }));

    // Only active item should have a ledger entry
    let active_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inventory_ledger WHERE tenant_id = $1 AND item_id = $2",
    )
    .bind(&tenant_id)
    .bind(active.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(active_count, 1);

    let inactive_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM inventory_ledger WHERE tenant_id = $1 AND item_id = $2",
    )
    .bind(&tenant_id)
    .bind(inactive.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(inactive_count, 0);

    cleanup(&pool, &tenant_id).await;
}

// ============================================================================
// Idempotency: replaying a batch returns stored results
// ============================================================================

#[tokio::test]
#[serial]
async fn batch_receipt_idempotency_replay_returns_stored_results() {
    let pool = setup_db().await;
    let tenant_id = format!("test-{}", Uuid::new_v4());

    let item1 = ItemRepo::create(&pool, &create_item_req(&tenant_id, "SKU-IDEM-BATCH-001"))
        .await
        .expect("create item 1");
    let item2 = ItemRepo::create(&pool, &create_item_req(&tenant_id, "SKU-IDEM-BATCH-002"))
        .await
        .expect("create item 2");

    let idem1 = format!("idem-{}", Uuid::new_v4());
    let idem2 = format!("idem-{}", Uuid::new_v4());

    let batch = BatchReceiptRequest {
        receipts: vec![
            receipt_req(&tenant_id, item1.id, &idem1),
            receipt_req(&tenant_id, item2.id, &idem2),
        ],
    };

    let results1: Vec<BatchReceiptItemResult> = process_batch(&pool, &batch).await;

    // Replay with same idempotency keys
    let results2: Vec<BatchReceiptItemResult> = process_batch(&pool, &batch).await;

    assert_eq!(results1.len(), 2);
    assert_eq!(results2.len(), 2);

    // Event IDs should match (idempotent replay)
    match (&results1[0], &results2[0]) {
        (BatchReceiptItemResult::Success(r1), BatchReceiptItemResult::Success(r2)) => {
            assert_eq!(r1.event_id, r2.event_id);
            assert_eq!(r1.receipt_line_id, r2.receipt_line_id);
        }
        _ => panic!("Expected success results"),
    }
    match (&results1[1], &results2[1]) {
        (BatchReceiptItemResult::Success(r1), BatchReceiptItemResult::Success(r2)) => {
            assert_eq!(r1.event_id, r2.event_id);
            assert_eq!(r1.receipt_line_id, r2.receipt_line_id);
        }
        _ => panic!("Expected success results"),
    }

    // Still only 2 ledger entries (no duplicates)
    let total: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM inventory_ledger WHERE tenant_id = $1")
            .bind(&tenant_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(total, 2);

    cleanup(&pool, &tenant_id).await;
}
