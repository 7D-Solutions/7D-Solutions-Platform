//! Phase A integration tests: source_type + production receipt + make/buy.
//!
//! Tests run against a real PostgreSQL database.
//! Set DATABASE_URL to the inventory database connection string.
//!
//! Coverage:
//! 1. Purchase receipt regression (source_type=purchase default)
//! 2. Production receipt path (source_type=production)
//! 3. Return receipt path (source_type=return)
//! 4. Invalid source_type rejected
//! 5. source_type persisted in ledger
//! 6. Make/buy set on item
//! 7. Make/buy change emits event
//! 8. Make/buy tenant isolation

use inventory_rs::domain::{
    items::{CreateItemRequest, ItemRepo, TrackingMode},
    make_buy::{set_make_buy, MakeBuyError, SetMakeBuyRequest},
    receipt_service::{process_receipt, ReceiptRequest},
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
        name: "Phase-A Test Item".to_string(),
        description: None,
        inventory_account_ref: "1200".to_string(),
        cogs_account_ref: "5000".to_string(),
        variance_account_ref: "5010".to_string(),
        uom: None,
        tracking_mode: TrackingMode::None,
        make_buy: None,
    }
}

fn receipt_req(
    tenant_id: &str,
    item_id: Uuid,
    source_type: &str,
    idem_key: &str,
) -> ReceiptRequest {
    ReceiptRequest {
        tenant_id: tenant_id.to_string(),
        item_id,
        warehouse_id: Uuid::new_v4(),
        quantity: 10,
        unit_cost_minor: 500,
        currency: "usd".to_string(),
        source_type: source_type.to_string(),
        purchase_order_id: None,
        idempotency_key: idem_key.to_string(),
        correlation_id: Some("phase-a-test".to_string()),
        causation_id: None,
        lot_code: None,
        serial_codes: None,
        location_id: None,
        uom_id: None,
    }
}

// ============================================================================
// 1. Purchase receipt regression — source_type defaults to "purchase"
// ============================================================================

#[tokio::test]
#[serial]
async fn purchase_receipt_regression() {
    let pool = setup_db().await;
    let tenant = format!("pa-purchase-{}", Uuid::new_v4());
    let item = ItemRepo::create(
        &pool,
        &create_item_req(&tenant, &format!("PUR-{}", Uuid::new_v4())),
    )
    .await
    .expect("create item");

    let req = receipt_req(
        &tenant,
        item.id,
        "purchase",
        &format!("pa-pur-{}", Uuid::new_v4()),
    );
    let wh = req.warehouse_id;
    let (result, _idempotent) = process_receipt(&pool, &req, None).await.expect("receipt");

    assert_eq!(result.source_type, "purchase");

    // Verify ledger row has source_type
    let ledger_source_type: String = sqlx::query_scalar(
        "SELECT source_type FROM inventory_ledger WHERE tenant_id = $1 AND item_id = $2 AND warehouse_id = $3 AND entry_type = 'received' LIMIT 1",
    )
    .bind(&tenant)
    .bind(item.id)
    .bind(wh)
    .fetch_one(&pool)
    .await
    .expect("fetch ledger");

    assert_eq!(ledger_source_type, "purchase");

    // Verify outbox event contains source_type
    let outbox_payload: serde_json::Value = sqlx::query_scalar(
        "SELECT payload FROM inv_outbox WHERE tenant_id = $1 AND event_type = 'inventory.item_received' ORDER BY created_at DESC LIMIT 1",
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .expect("fetch outbox");

    let payload_source = outbox_payload["payload"]["source_type"]
        .as_str()
        .expect("source_type in payload");
    assert_eq!(payload_source, "purchase");
}

// ============================================================================
// 2. Production receipt path
// ============================================================================

#[tokio::test]
#[serial]
async fn production_receipt_accepted() {
    let pool = setup_db().await;
    let tenant = format!("pa-prod-{}", Uuid::new_v4());
    let item = ItemRepo::create(
        &pool,
        &create_item_req(&tenant, &format!("PROD-{}", Uuid::new_v4())),
    )
    .await
    .expect("create item");

    let req = receipt_req(
        &tenant,
        item.id,
        "production",
        &format!("pa-prod-{}", Uuid::new_v4()),
    );
    let (result, _) = process_receipt(&pool, &req, None)
        .await
        .expect("production receipt");

    assert_eq!(result.source_type, "production");

    // Verify ledger
    let ledger_source_type: String = sqlx::query_scalar(
        "SELECT source_type FROM inventory_ledger WHERE tenant_id = $1 AND item_id = $2 AND entry_type = 'received' LIMIT 1",
    )
    .bind(&tenant)
    .bind(item.id)
    .fetch_one(&pool)
    .await
    .expect("fetch ledger");
    assert_eq!(ledger_source_type, "production");
}

// ============================================================================
// 3. Return receipt path
// ============================================================================

#[tokio::test]
#[serial]
async fn return_receipt_accepted() {
    let pool = setup_db().await;
    let tenant = format!("pa-ret-{}", Uuid::new_v4());
    let item = ItemRepo::create(
        &pool,
        &create_item_req(&tenant, &format!("RET-{}", Uuid::new_v4())),
    )
    .await
    .expect("create item");

    let req = receipt_req(
        &tenant,
        item.id,
        "return",
        &format!("pa-ret-{}", Uuid::new_v4()),
    );
    let (result, _) = process_receipt(&pool, &req, None)
        .await
        .expect("return receipt");

    assert_eq!(result.source_type, "return");
}

// ============================================================================
// 4. Invalid source_type rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn invalid_source_type_rejected() {
    let pool = setup_db().await;
    let tenant = format!("pa-inv-{}", Uuid::new_v4());
    let item = ItemRepo::create(
        &pool,
        &create_item_req(&tenant, &format!("INV-{}", Uuid::new_v4())),
    )
    .await
    .expect("create item");

    let req = receipt_req(
        &tenant,
        item.id,
        "invalid_type",
        &format!("pa-inv-{}", Uuid::new_v4()),
    );
    let result = process_receipt(&pool, &req, None).await;

    assert!(result.is_err(), "invalid source_type should be rejected");
    let err_msg = format!("{}", result.unwrap_err());
    assert!(
        err_msg.contains("source_type"),
        "error should mention source_type: {}",
        err_msg
    );
}

// ============================================================================
// 5. Make/buy — set classification
// ============================================================================

#[tokio::test]
#[serial]
async fn set_make_buy_classification() {
    let pool = setup_db().await;
    let tenant = format!("pa-mb-{}", Uuid::new_v4());
    let item = ItemRepo::create(
        &pool,
        &create_item_req(&tenant, &format!("MB-{}", Uuid::new_v4())),
    )
    .await
    .expect("create item");

    // Initially null
    assert!(item.make_buy.is_none());

    // Set to "make"
    let req = SetMakeBuyRequest {
        tenant_id: tenant.clone(),
        make_buy: "make".to_string(),
        correlation_id: Some("pa-test".to_string()),
        causation_id: None,
    };
    let result = set_make_buy(&pool, item.id, &req)
        .await
        .expect("set make_buy");
    assert_eq!(result.item.make_buy.as_deref(), Some("make"));
    assert!(result.previous_value.is_none());

    // Change to "buy"
    let req2 = SetMakeBuyRequest {
        tenant_id: tenant.clone(),
        make_buy: "buy".to_string(),
        correlation_id: Some("pa-test".to_string()),
        causation_id: None,
    };
    let result2 = set_make_buy(&pool, item.id, &req2)
        .await
        .expect("change make_buy");
    assert_eq!(result2.item.make_buy.as_deref(), Some("buy"));
    assert_eq!(result2.previous_value.as_deref(), Some("make"));
}

// ============================================================================
// 6. Make/buy — outbox event emitted
// ============================================================================

#[tokio::test]
#[serial]
async fn make_buy_emits_outbox_event() {
    let pool = setup_db().await;
    let tenant = format!("pa-mb-evt-{}", Uuid::new_v4());
    let item = ItemRepo::create(
        &pool,
        &create_item_req(&tenant, &format!("MBE-{}", Uuid::new_v4())),
    )
    .await
    .expect("create item");

    let req = SetMakeBuyRequest {
        tenant_id: tenant.clone(),
        make_buy: "buy".to_string(),
        correlation_id: Some("pa-evt-test".to_string()),
        causation_id: None,
    };
    set_make_buy(&pool, item.id, &req)
        .await
        .expect("set make_buy");

    let outbox_payload: serde_json::Value = sqlx::query_scalar(
        "SELECT payload FROM inv_outbox WHERE tenant_id = $1 AND event_type = 'inventory.make_buy_changed' ORDER BY created_at DESC LIMIT 1",
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .expect("fetch outbox event");

    let payload = &outbox_payload["payload"];
    assert_eq!(payload["new_value"].as_str(), Some("buy"));
    assert!(payload["previous_value"].is_null());
    assert_eq!(
        payload["item_id"].as_str(),
        Some(item.id.to_string().as_str())
    );
}

// ============================================================================
// 7. Make/buy — invalid value rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn make_buy_invalid_rejected() {
    let pool = setup_db().await;
    let tenant = format!("pa-mb-inv-{}", Uuid::new_v4());
    let item = ItemRepo::create(
        &pool,
        &create_item_req(&tenant, &format!("MBI-{}", Uuid::new_v4())),
    )
    .await
    .expect("create item");

    let req = SetMakeBuyRequest {
        tenant_id: tenant.clone(),
        make_buy: "manufacture".to_string(),
        correlation_id: None,
        causation_id: None,
    };
    let result = set_make_buy(&pool, item.id, &req).await;
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), MakeBuyError::Validation(_)));
}

// ============================================================================
// 8. Make/buy — tenant isolation
// ============================================================================

#[tokio::test]
#[serial]
async fn make_buy_tenant_isolation() {
    let pool = setup_db().await;
    let tenant_a = format!("pa-iso-a-{}", Uuid::new_v4());
    let tenant_b = format!("pa-iso-b-{}", Uuid::new_v4());

    let item_a = ItemRepo::create(
        &pool,
        &create_item_req(&tenant_a, &format!("ISO-A-{}", Uuid::new_v4())),
    )
    .await
    .expect("create item a");

    // Set make/buy on tenant_a's item
    let req = SetMakeBuyRequest {
        tenant_id: tenant_a.clone(),
        make_buy: "make".to_string(),
        correlation_id: None,
        causation_id: None,
    };
    set_make_buy(&pool, item_a.id, &req)
        .await
        .expect("set make_buy on item_a");

    // Tenant B should not be able to set make_buy on tenant_a's item
    let cross_req = SetMakeBuyRequest {
        tenant_id: tenant_b.clone(),
        make_buy: "buy".to_string(),
        correlation_id: None,
        causation_id: None,
    };
    let result = set_make_buy(&pool, item_a.id, &cross_req).await;
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), MakeBuyError::NotFound));
}

// ============================================================================
// 9. Create item with make_buy preset
// ============================================================================

#[tokio::test]
#[serial]
async fn create_item_with_make_buy() {
    let pool = setup_db().await;
    let tenant = format!("pa-create-mb-{}", Uuid::new_v4());
    let mut req = create_item_req(&tenant, &format!("CMB-{}", Uuid::new_v4()));
    req.make_buy = Some("buy".to_string());

    let item = ItemRepo::create(&pool, &req)
        .await
        .expect("create item with make_buy");
    assert_eq!(item.make_buy.as_deref(), Some("buy"));
}
