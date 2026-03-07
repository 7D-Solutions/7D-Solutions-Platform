//! Integrated E2E: Inventory Extensions Suite (bd-51b5)
//!
//! End-cap proof for W4-W6 inventory extensions. One cohesive file covering
//! lot/serial tracking, cycle count approve, status buckets, UoM conversions,
//! valuation snapshots, and low-stock signal dedup.
//!
//! ## Coverage
//! 1. lot_tracked_flow               — receipt with lot_code; issue requires lot_code
//! 2. serial_tracked_flow            — receipt with serial_codes; issue requires serial_codes
//! 3. cycle_count_full_scope         — create/submit/approve → adjustments written
//! 4. status_buckets                 — quarantine removes from available; transfer restores
//! 5. uom_receipt_issue              — non-base UoM receipt/issue converts correctly
//! 6. valuation_snapshot_matches_fifo — snapshot value = remaining FIFO layer value
//! 7. low_stock_dedup                — threshold crossing emits one signal until re-arm
//!
//! ## Services
//! Inventory DB only (INVENTORY_DATABASE_URL or DATABASE_URL → localhost:5442).
//! Low-stock test also uses notifications DB (NOTIFICATIONS_DATABASE_URL → localhost:5437).
//!
//! ## No mocks, no stubs, no Docker spin-up inside tests.

mod common;

use chrono::Utc;
use common::generate_test_tenant;
use inventory_rs::domain::{
    cycle_count::{
        approve_service::{approve_cycle_count, ApproveRequest},
        submit_service::{submit_cycle_count, SubmitLineInput, SubmitRequest},
        task_service::{create_cycle_count_task, CreateTaskRequest, TaskScope},
    },
    issue_service::{process_issue, IssueRequest},
    items::{CreateItemRequest, ItemRepo, TrackingMode},
    locations::{CreateLocationRequest, LocationRepo},
    receipt_service::{process_receipt, ReceiptRequest},
    reorder::models::{CreateReorderPolicyRequest, ReorderPolicyRepo},
    status::{
        models::InvItemStatus,
        transfer_service::{process_status_transfer, StatusTransferRequest},
    },
    uom::models::{ConversionRepo, CreateConversionRequest, CreateUomRequest, UomRepo},
    valuation::{
        queries::{get_snapshot, get_snapshot_lines},
        snapshot_service::{create_valuation_snapshot, CreateSnapshotRequest},
    },
};
use notifications_rs::{
    handlers::handle_low_stock_triggered,
    models::{EnvelopeMetadata, LowStockTriggeredPayload},
};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

// ============================================================================
// Pool helpers
// ============================================================================

async fn get_inventory_pool() -> sqlx::PgPool {
    let url = std::env::var("INVENTORY_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| {
            "postgresql://inventory_user:inventory_pass@localhost:5442/inventory_db".to_string()
        });
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to inventory DB");
    sqlx::migrate!("../modules/inventory/db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run inventory migrations");
    pool
}

async fn get_notifications_pool() -> sqlx::PgPool {
    let url = std::env::var("NOTIFICATIONS_DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://notifications_user:notifications_pass@localhost:5437/notifications_db"
            .to_string()
    });
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to notifications DB");
    sqlx::migrate!("../modules/notifications/db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run notifications migrations");
    pool
}

// ============================================================================
// Shared helpers
// ============================================================================

fn item_req(tenant_id: &str, sku: &str, tracking: TrackingMode) -> CreateItemRequest {
    CreateItemRequest {
        tenant_id: tenant_id.to_string(),
        sku: sku.to_string(),
        name: format!("Ext-Test {}", sku),
        description: None,
        inventory_account_ref: "1200".to_string(),
        cogs_account_ref: "5000".to_string(),
        variance_account_ref: "5010".to_string(),
        uom: None,
        tracking_mode: tracking,
        make_buy: None,
    }
}

fn key() -> String {
    format!("ext-{}", Uuid::new_v4())
}

async fn cleanup(pool: &sqlx::PgPool, tenant_id: &str) {
    for q in [
        "DELETE FROM inv_outbox WHERE tenant_id = $1",
        "DELETE FROM inv_idempotency_keys WHERE tenant_id = $1",
        "DELETE FROM reorder_signal_state WHERE tenant_id = $1",
        "DELETE FROM reorder_policies WHERE tenant_id = $1",
        "DELETE FROM inventory_valuation_lines WHERE snapshot_id IN \
             (SELECT id FROM inventory_valuation_snapshots WHERE tenant_id = $1)",
        "DELETE FROM inventory_valuation_snapshots WHERE tenant_id = $1",
        "DELETE FROM inv_status_transfers WHERE tenant_id = $1",
        "DELETE FROM item_on_hand_by_status WHERE tenant_id = $1",
        "DELETE FROM cycle_count_lines WHERE tenant_id = $1",
        "DELETE FROM cycle_count_tasks WHERE tenant_id = $1",
        "DELETE FROM inv_adjustments WHERE tenant_id = $1",
        "DELETE FROM inventory_reservations WHERE tenant_id = $1",
        "DELETE FROM layer_consumptions WHERE ledger_entry_id IN \
             (SELECT id FROM inventory_ledger WHERE tenant_id = $1)",
        "DELETE FROM inventory_serial_instances WHERE tenant_id = $1",
        "DELETE FROM inventory_lots WHERE tenant_id = $1",
        "DELETE FROM item_on_hand WHERE tenant_id = $1",
        "DELETE FROM inventory_layers WHERE tenant_id = $1",
        "DELETE FROM inventory_ledger WHERE tenant_id = $1",
        "DELETE FROM item_uom_conversions WHERE tenant_id = $1",
        "DELETE FROM uoms WHERE tenant_id = $1",
        "DELETE FROM locations WHERE tenant_id = $1",
        "DELETE FROM items WHERE tenant_id = $1",
    ] {
        sqlx::query(q).bind(tenant_id).execute(pool).await.ok();
    }
}

// ============================================================================
// Test 1: Lot-tracked flow
// ============================================================================

/// Lot-tracked items: receipt with lot_code succeeds; issue with lot_code
/// succeeds; issue without lot_code returns an error.
#[tokio::test]
#[serial]
async fn lot_tracked_flow() {
    let pool = get_inventory_pool().await;
    let tenant = generate_test_tenant();
    let wh = Uuid::new_v4();

    let item = ItemRepo::create(&pool, &item_req(&tenant, "LOT-EXT-001", TrackingMode::Lot))
        .await
        .expect("create lot item");

    // Receipt with lot_code
    process_receipt(
        &pool,
        &ReceiptRequest {
            tenant_id: tenant.clone(),
            item_id: item.id,
            warehouse_id: wh,
            location_id: None,
            quantity: 20,
            unit_cost_minor: 500,
            currency: "usd".to_string(),
            source_type: "purchase".to_string(),
            purchase_order_id: None,
            idempotency_key: key(),
            correlation_id: None,
            causation_id: None,
            lot_code: Some("LOT-2026-01".to_string()),
            serial_codes: None,
            uom_id: None,
        },
        None,
    )
    .await
    .expect("lot receipt");

    // Issue WITH lot_code — must succeed
    let result = process_issue(
        &pool,
        &IssueRequest {
            tenant_id: tenant.clone(),
            item_id: item.id,
            warehouse_id: wh,
            location_id: None,
            quantity: 5,
            currency: "usd".to_string(),
            source_module: "test".to_string(),
            source_type: "test_order".to_string(),
            source_id: "ORD-001".to_string(),
            source_line_id: None,
            idempotency_key: key(),
            correlation_id: None,
            causation_id: None,
            uom_id: None,
            lot_code: Some("LOT-2026-01".to_string()),
            serial_codes: None,
        },
        None,
    )
    .await;
    assert!(
        result.is_ok(),
        "lot issue with lot_code must succeed: {:?}",
        result.err()
    );
    assert_eq!(result.unwrap().0.quantity, 5);

    // Issue WITHOUT lot_code — must fail
    let bad = process_issue(
        &pool,
        &IssueRequest {
            tenant_id: tenant.clone(),
            item_id: item.id,
            warehouse_id: wh,
            location_id: None,
            quantity: 3,
            currency: "usd".to_string(),
            source_module: "test".to_string(),
            source_type: "test_order".to_string(),
            source_id: "ORD-002".to_string(),
            source_line_id: None,
            idempotency_key: key(),
            correlation_id: None,
            causation_id: None,
            uom_id: None,
            lot_code: None,
            serial_codes: None,
        },
        None,
    )
    .await;
    assert!(
        bad.is_err(),
        "issue without lot_code on lot-tracked item must fail"
    );

    cleanup(&pool, &tenant).await;
}

// ============================================================================
// Test 2: Serial-tracked flow
// ============================================================================

/// Serial-tracked items: receipt registers serials; issue with matching codes
/// succeeds; issue without serial_codes returns an error.
#[tokio::test]
#[serial]
async fn serial_tracked_flow() {
    let pool = get_inventory_pool().await;
    let tenant = generate_test_tenant();
    let wh = Uuid::new_v4();

    let item = ItemRepo::create(
        &pool,
        &item_req(&tenant, "SER-EXT-001", TrackingMode::Serial),
    )
    .await
    .expect("create serial item");

    let serials = vec![
        "SN-001".to_string(),
        "SN-002".to_string(),
        "SN-003".to_string(),
    ];

    // Receipt with serial_codes
    process_receipt(
        &pool,
        &ReceiptRequest {
            tenant_id: tenant.clone(),
            item_id: item.id,
            warehouse_id: wh,
            location_id: None,
            quantity: 3,
            unit_cost_minor: 1_000,
            currency: "usd".to_string(),
            source_type: "purchase".to_string(),
            purchase_order_id: None,
            idempotency_key: key(),
            correlation_id: None,
            causation_id: None,
            lot_code: None,
            serial_codes: Some(serials.clone()),
            uom_id: None,
        },
        None,
    )
    .await
    .expect("serial receipt");

    // Issue WITH serial_codes — must succeed
    let result = process_issue(
        &pool,
        &IssueRequest {
            tenant_id: tenant.clone(),
            item_id: item.id,
            warehouse_id: wh,
            location_id: None,
            quantity: 2, // ignored for serial; derived from serial_codes.len()
            currency: "usd".to_string(),
            source_module: "test".to_string(),
            source_type: "test_order".to_string(),
            source_id: "ORD-SER-001".to_string(),
            source_line_id: None,
            idempotency_key: key(),
            correlation_id: None,
            causation_id: None,
            uom_id: None,
            lot_code: None,
            serial_codes: Some(vec!["SN-001".to_string(), "SN-002".to_string()]),
        },
        None,
    )
    .await;
    assert!(
        result.is_ok(),
        "serial issue with codes must succeed: {:?}",
        result.err()
    );
    assert_eq!(result.unwrap().0.quantity, 2);

    // Issue WITHOUT serial_codes — must fail
    let bad = process_issue(
        &pool,
        &IssueRequest {
            tenant_id: tenant.clone(),
            item_id: item.id,
            warehouse_id: wh,
            location_id: None,
            quantity: 1,
            currency: "usd".to_string(),
            source_module: "test".to_string(),
            source_type: "test_order".to_string(),
            source_id: "ORD-SER-002".to_string(),
            source_line_id: None,
            idempotency_key: key(),
            correlation_id: None,
            causation_id: None,
            uom_id: None,
            lot_code: None,
            serial_codes: None,
        },
        None,
    )
    .await;
    assert!(bad.is_err(), "serial issue without serial_codes must fail");

    cleanup(&pool, &tenant).await;
}

// ============================================================================
// Test 3: Cycle count full scope — create → submit → approve → adjustments
// ============================================================================

/// Full-scope cycle count: count lower than expected → approve creates an
/// adjustment row in inv_adjustments for the variance.
#[tokio::test]
#[serial]
async fn cycle_count_full_scope() {
    let pool = get_inventory_pool().await;
    let tenant = generate_test_tenant();
    let wh = Uuid::new_v4();

    // Create item and location
    let item = ItemRepo::create(&pool, &item_req(&tenant, "CC-EXT-001", TrackingMode::None))
        .await
        .expect("create item");

    let loc = LocationRepo::create(
        &pool,
        &CreateLocationRequest {
            tenant_id: tenant.clone(),
            warehouse_id: wh,
            code: "BIN-A".to_string(),
            name: "Bin A".to_string(),
            description: None,
        },
    )
    .await
    .expect("create location");

    // Receipt 50 units to this specific location
    process_receipt(
        &pool,
        &ReceiptRequest {
            tenant_id: tenant.clone(),
            item_id: item.id,
            warehouse_id: wh,
            location_id: Some(loc.id),
            quantity: 50,
            unit_cost_minor: 200,
            currency: "usd".to_string(),
            source_type: "purchase".to_string(),
            purchase_order_id: None,
            idempotency_key: key(),
            correlation_id: None,
            causation_id: None,
            lot_code: None,
            serial_codes: None,
            uom_id: None,
        },
        None,
    )
    .await
    .expect("receipt to location");

    // Create full-scope cycle count task
    let task = create_cycle_count_task(
        &pool,
        &CreateTaskRequest {
            tenant_id: tenant.clone(),
            warehouse_id: wh,
            location_id: loc.id,
            scope: TaskScope::Full,
            item_ids: vec![],
        },
    )
    .await
    .expect("create cycle count task");

    assert_eq!(task.line_count, 1, "one item with stock at this location");
    assert_eq!(task.lines[0].item_id, item.id);
    assert_eq!(task.lines[0].expected_qty, 50);

    let line_id = task.lines[0].line_id;

    // Submit with counted_qty = 45 (variance = -5, shrinkage)
    let submit_req = SubmitRequest {
        task_id: task.task_id,
        tenant_id: tenant.clone(),
        idempotency_key: key(),
        lines: vec![SubmitLineInput {
            line_id,
            counted_qty: 45,
        }],
        correlation_id: None,
        causation_id: None,
    };
    let (submit_result, _) = submit_cycle_count(&pool, &submit_req)
        .await
        .expect("submit cycle count");

    assert_eq!(submit_result.status, "submitted");
    assert_eq!(submit_result.lines[0].variance_qty, -5);

    // Approve — creates one adjustment for the -5 variance
    let approve_req = ApproveRequest {
        task_id: task.task_id,
        tenant_id: tenant.clone(),
        idempotency_key: key(),
        correlation_id: None,
        causation_id: None,
    };
    let (approve_result, _) = approve_cycle_count(&pool, &approve_req)
        .await
        .expect("approve cycle count");

    assert_eq!(approve_result.status, "approved");
    assert_eq!(
        approve_result.adjustment_count, 1,
        "one non-zero variance line → one adjustment"
    );

    // Verify the adjustment row exists in inv_adjustments
    let adj_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM inv_adjustments WHERE tenant_id = $1")
            .bind(&tenant)
            .fetch_one(&pool)
            .await
            .expect("query adjustments");
    assert_eq!(adj_count, 1, "one adjustment row created");

    cleanup(&pool, &tenant).await;
}

// ============================================================================
// Test 4: Status buckets — quarantine removes available; transfer restores
// ============================================================================

/// After quarantining all stock, `quantity_available = 0` (no reservable
/// stock). After status transfer back to available, `quantity_available`
/// is restored to the original quantity.
#[tokio::test]
#[serial]
async fn status_buckets_quarantine_and_restore() {
    let pool = get_inventory_pool().await;
    let tenant = generate_test_tenant();
    let wh = Uuid::new_v4();

    let item = ItemRepo::create(&pool, &item_req(&tenant, "SB-EXT-001", TrackingMode::None))
        .await
        .expect("create item");

    // Receipt 30 units (goes to 'available' bucket)
    process_receipt(
        &pool,
        &ReceiptRequest {
            tenant_id: tenant.clone(),
            item_id: item.id,
            warehouse_id: wh,
            location_id: None,
            quantity: 30,
            unit_cost_minor: 100,
            currency: "usd".to_string(),
            source_type: "purchase".to_string(),
            purchase_order_id: None,
            idempotency_key: key(),
            correlation_id: None,
            causation_id: None,
            lot_code: None,
            serial_codes: None,
            uom_id: None,
        },
        None,
    )
    .await
    .expect("receipt");

    // Confirm all 30 units are in available bucket
    let available_before: i64 = sqlx::query_scalar(
        "SELECT quantity_available FROM item_on_hand \
         WHERE tenant_id = $1 AND item_id = $2 AND warehouse_id = $3 \
         AND location_id IS NULL",
    )
    .bind(&tenant)
    .bind(item.id)
    .bind(wh)
    .fetch_one(&pool)
    .await
    .expect("query available before");
    assert_eq!(
        available_before, 30,
        "all 30 units should be available initially"
    );

    // Quarantine all 30 units: available → quarantine
    let (transfer_result, _) = process_status_transfer(
        &pool,
        &StatusTransferRequest {
            tenant_id: tenant.clone(),
            item_id: item.id,
            warehouse_id: wh,
            from_status: InvItemStatus::Available,
            to_status: InvItemStatus::Quarantine,
            quantity: 30,
            idempotency_key: key(),
            correlation_id: None,
            causation_id: None,
        },
    )
    .await
    .expect("quarantine transfer");

    assert_eq!(transfer_result.from_status, "available");
    assert_eq!(transfer_result.to_status, "quarantine");
    assert_eq!(transfer_result.quantity, 30);

    // After quarantine: quantity_available must be 0 (no reservable stock)
    let available_after_quarantine: i64 = sqlx::query_scalar(
        "SELECT quantity_available FROM item_on_hand \
         WHERE tenant_id = $1 AND item_id = $2 AND warehouse_id = $3 \
         AND location_id IS NULL",
    )
    .bind(&tenant)
    .bind(item.id)
    .bind(wh)
    .fetch_one(&pool)
    .await
    .expect("query available after quarantine");
    assert_eq!(
        available_after_quarantine, 0,
        "quarantined stock cannot be reserved: quantity_available must be 0"
    );

    // Quarantine bucket should now have 30 units
    let quarantine_qty: i64 = sqlx::query_scalar(
        "SELECT quantity_on_hand FROM item_on_hand_by_status \
         WHERE tenant_id = $1 AND item_id = $2 AND warehouse_id = $3 AND status = 'quarantine'",
    )
    .bind(&tenant)
    .bind(item.id)
    .bind(wh)
    .fetch_one(&pool)
    .await
    .expect("query quarantine bucket");
    assert_eq!(
        quarantine_qty, 30,
        "quarantine bucket holds the transferred qty"
    );

    // Status transfer back: quarantine → available (restore)
    process_status_transfer(
        &pool,
        &StatusTransferRequest {
            tenant_id: tenant.clone(),
            item_id: item.id,
            warehouse_id: wh,
            from_status: InvItemStatus::Quarantine,
            to_status: InvItemStatus::Available,
            quantity: 30,
            idempotency_key: key(),
            correlation_id: None,
            causation_id: None,
        },
    )
    .await
    .expect("restore transfer");

    // After restore: quantity_available must be 30 again
    let available_restored: i64 = sqlx::query_scalar(
        "SELECT quantity_available FROM item_on_hand \
         WHERE tenant_id = $1 AND item_id = $2 AND warehouse_id = $3 \
         AND location_id IS NULL",
    )
    .bind(&tenant)
    .bind(item.id)
    .bind(wh)
    .fetch_one(&pool)
    .await
    .expect("query available after restore");
    assert_eq!(
        available_restored, 30,
        "availability restored after moving stock back to available bucket"
    );

    cleanup(&pool, &tenant).await;
}

// ============================================================================
// Test 5: UoM receipt/issue — non-base UoM converted correctly
// ============================================================================

/// Receipt 2 boxes (1 box = 12 ea) → stored as 24 ea.
/// Issue 1 box → stored as 12 ea consumed.
/// Projection: 12 ea remain.
#[tokio::test]
#[serial]
async fn uom_receipt_issue_converts_correctly() {
    let pool = get_inventory_pool().await;
    let tenant = generate_test_tenant();
    let wh = Uuid::new_v4();

    // Create UoMs
    let uom_ea = UomRepo::create(
        &pool,
        &CreateUomRequest {
            tenant_id: tenant.clone(),
            code: format!("ea-{}", Uuid::new_v4()),
            name: "Each".to_string(),
        },
    )
    .await
    .expect("create ea UoM");

    let uom_box = UomRepo::create(
        &pool,
        &CreateUomRequest {
            tenant_id: tenant.clone(),
            code: format!("box-{}", Uuid::new_v4()),
            name: "Box".to_string(),
        },
    )
    .await
    .expect("create box UoM");

    // Create item (base UoM = ea)
    // ItemRepo::create stores uom as a string label; base_uom_id is a separate
    // UUID column that must be set explicitly to enable UoM conversion guards.
    let item = ItemRepo::create(
        &pool,
        &CreateItemRequest {
            tenant_id: tenant.clone(),
            sku: "UOM-EXT-001".to_string(),
            name: "UoM Test Widget".to_string(),
            description: None,
            inventory_account_ref: "1200".to_string(),
            cogs_account_ref: "5000".to_string(),
            variance_account_ref: "5010".to_string(),
            uom: None, // string label (not used by guard_convert_to_base)
            tracking_mode: TrackingMode::None,
            make_buy: None,
        },
    )
    .await
    .expect("create item");

    // Set base_uom_id — the UUID FK used by guard_convert_to_base.
    // The application layer does not expose a dedicated endpoint for this yet;
    // set it directly so the conversion guard can resolve the canonical unit.
    sqlx::query("UPDATE items SET base_uom_id = $1 WHERE id = $2")
        .bind(uom_ea.id)
        .bind(item.id)
        .execute(&pool)
        .await
        .expect("set base_uom_id on item");

    // Add conversion: 1 box = 12 ea
    ConversionRepo::create(
        &pool,
        item.id,
        &CreateConversionRequest {
            tenant_id: tenant.clone(),
            from_uom_id: uom_box.id,
            to_uom_id: uom_ea.id,
            factor: 12.0,
        },
    )
    .await
    .expect("create box→ea conversion");

    // Receipt 2 boxes (2 * 12 = 24 ea stored)
    process_receipt(
        &pool,
        &ReceiptRequest {
            tenant_id: tenant.clone(),
            item_id: item.id,
            warehouse_id: wh,
            location_id: None,
            quantity: 2,
            unit_cost_minor: 600, // per box = 50 per ea after conversion
            currency: "usd".to_string(),
            source_type: "purchase".to_string(),
            purchase_order_id: None,
            idempotency_key: key(),
            correlation_id: None,
            causation_id: None,
            lot_code: None,
            serial_codes: None,
            uom_id: Some(uom_box.id),
        },
        None,
    )
    .await
    .expect("receipt 2 boxes");

    // Check projection: 24 ea on hand
    let on_hand: i64 = sqlx::query_scalar(
        "SELECT quantity_on_hand FROM item_on_hand \
         WHERE tenant_id = $1 AND item_id = $2 AND warehouse_id = $3 \
         AND location_id IS NULL",
    )
    .bind(&tenant)
    .bind(item.id)
    .bind(wh)
    .fetch_one(&pool)
    .await
    .expect("query on_hand after receipt");
    assert_eq!(on_hand, 24, "2 boxes × 12 ea/box = 24 ea on hand");

    // Issue 1 box (= 12 ea)
    let (issue_result, _) = process_issue(
        &pool,
        &IssueRequest {
            tenant_id: tenant.clone(),
            item_id: item.id,
            warehouse_id: wh,
            location_id: None,
            quantity: 1,
            currency: "usd".to_string(),
            source_module: "test".to_string(),
            source_type: "test_order".to_string(),
            source_id: "ORD-UOM-001".to_string(),
            source_line_id: None,
            idempotency_key: key(),
            correlation_id: None,
            causation_id: None,
            uom_id: Some(uom_box.id),
            lot_code: None,
            serial_codes: None,
        },
        None,
    )
    .await
    .expect("issue 1 box");

    assert_eq!(
        issue_result.quantity, 12,
        "issued qty stored in base units (ea)"
    );

    // Projection should show 12 ea remaining
    let on_hand_after: i64 = sqlx::query_scalar(
        "SELECT quantity_on_hand FROM item_on_hand \
         WHERE tenant_id = $1 AND item_id = $2 AND warehouse_id = $3 \
         AND location_id IS NULL",
    )
    .bind(&tenant)
    .bind(item.id)
    .bind(wh)
    .fetch_one(&pool)
    .await
    .expect("query on_hand after issue");
    assert_eq!(on_hand_after, 12, "12 ea remain after issuing 1 box");

    cleanup(&pool, &tenant).await;
}

// ============================================================================
// Test 6: Valuation snapshot matches remaining FIFO layer value
// ============================================================================

/// Receipt 10 units @ 500 minor each, then issue 4.
/// Remaining: 6 units × 500 = 3000 minor.
/// Valuation snapshot total_value_minor must equal 3000.
#[tokio::test]
#[serial]
async fn valuation_snapshot_matches_fifo() {
    let pool = get_inventory_pool().await;
    let tenant = generate_test_tenant();
    let wh = Uuid::new_v4();

    let item = ItemRepo::create(&pool, &item_req(&tenant, "VAL-EXT-001", TrackingMode::None))
        .await
        .expect("create item");

    // Receipt 10 units @ 500
    process_receipt(
        &pool,
        &ReceiptRequest {
            tenant_id: tenant.clone(),
            item_id: item.id,
            warehouse_id: wh,
            location_id: None,
            quantity: 10,
            unit_cost_minor: 500,
            currency: "usd".to_string(),
            source_type: "purchase".to_string(),
            purchase_order_id: None,
            idempotency_key: key(),
            correlation_id: None,
            causation_id: None,
            lot_code: None,
            serial_codes: None,
            uom_id: None,
        },
        None,
    )
    .await
    .expect("receipt 10 units");

    // Issue 4 units (consumed from FIFO layer)
    process_issue(
        &pool,
        &IssueRequest {
            tenant_id: tenant.clone(),
            item_id: item.id,
            warehouse_id: wh,
            location_id: None,
            quantity: 4,
            currency: "usd".to_string(),
            source_module: "test".to_string(),
            source_type: "test_order".to_string(),
            source_id: "ORD-VAL-001".to_string(),
            source_line_id: None,
            idempotency_key: key(),
            correlation_id: None,
            causation_id: None,
            uom_id: None,
            lot_code: None,
            serial_codes: None,
        },
        None,
    )
    .await
    .expect("issue 4 units");

    // Create valuation snapshot
    let snap_req = CreateSnapshotRequest {
        tenant_id: tenant.clone(),
        warehouse_id: wh,
        location_id: None,
        as_of: Utc::now(),
        idempotency_key: key(),
        currency: "usd".to_string(),
        correlation_id: None,
        causation_id: None,
    };
    let (snap_result, _) = create_valuation_snapshot(&pool, &snap_req)
        .await
        .expect("create valuation snapshot");

    // Snapshot header: 6 remaining × 500 = 3000
    assert_eq!(
        snap_result.total_value_minor, 3_000,
        "snapshot value = 6 remaining units × 500 minor/unit"
    );

    // Fetch snapshot header via query — tenant-scoped
    let header = get_snapshot(&pool, &tenant, snap_result.snapshot_id)
        .await
        .expect("get snapshot")
        .expect("snapshot must exist");
    assert_eq!(header.id, snap_result.snapshot_id);
    assert_eq!(header.total_value_minor, 3_000);

    // Fetch lines
    let lines = get_snapshot_lines(&pool, snap_result.snapshot_id)
        .await
        .expect("get snapshot lines");
    assert_eq!(lines.len(), 1, "one item → one line");
    assert_eq!(lines[0].item_id, item.id);
    assert_eq!(lines[0].quantity_on_hand, 6);
    assert_eq!(lines[0].unit_cost_minor, 500);
    assert_eq!(lines[0].total_value_minor, 3_000);

    cleanup(&pool, &tenant).await;
}

// ============================================================================
// Test 7: Low-stock dedup — one signal until recovery, then re-arm
// ============================================================================

/// Crosses threshold → one low_stock_triggered in inv_outbox.
/// Second issue while still below threshold → no second signal (dedup).
/// Adjustment drives qty above threshold → resets state.
/// Third below-threshold crossing → new signal (re-arm confirmed).
#[tokio::test]
#[serial]
async fn low_stock_dedup_and_rearm() {
    let inv_pool = get_inventory_pool().await;
    let notif_pool = get_notifications_pool().await;
    let tenant = generate_test_tenant();
    let wh = Uuid::new_v4();

    let item = ItemRepo::create(
        &inv_pool,
        &item_req(&tenant, "LS-EXT-001", TrackingMode::None),
    )
    .await
    .expect("create item");

    // Create reorder policy: reorder_point = 10
    ReorderPolicyRepo::create(
        &inv_pool,
        &CreateReorderPolicyRequest {
            tenant_id: tenant.clone(),
            item_id: item.id,
            location_id: None,
            reorder_point: 10,
            safety_stock: 0,
            max_qty: None,
            notes: None,
            created_by: None,
        },
    )
    .await
    .expect("create reorder policy");

    // Receipt 20 units (above threshold)
    process_receipt(
        &inv_pool,
        &ReceiptRequest {
            tenant_id: tenant.clone(),
            item_id: item.id,
            warehouse_id: wh,
            location_id: None,
            quantity: 20,
            unit_cost_minor: 1_000,
            currency: "usd".to_string(),
            source_type: "purchase".to_string(),
            purchase_order_id: None,
            idempotency_key: key(),
            correlation_id: None,
            causation_id: None,
            lot_code: None,
            serial_codes: None,
            uom_id: None,
        },
        None,
    )
    .await
    .expect("receipt");

    // Issue 15 → drops below reorder_point (5 remaining < 10)
    process_issue(
        &inv_pool,
        &IssueRequest {
            tenant_id: tenant.clone(),
            item_id: item.id,
            warehouse_id: wh,
            location_id: None,
            quantity: 15,
            currency: "usd".to_string(),
            source_module: "test".to_string(),
            source_type: "test_order".to_string(),
            source_id: "LS-ORD-001".to_string(),
            source_line_id: None,
            idempotency_key: key(),
            correlation_id: None,
            causation_id: None,
            uom_id: None,
            lot_code: None,
            serial_codes: None,
        },
        None,
    )
    .await
    .expect("first issue");

    // One low_stock signal should be in inv_outbox
    let count_1 = count_low_stock_outbox(&inv_pool, &tenant, item.id).await;
    assert_eq!(count_1, 1, "first threshold crossing emits one signal");

    // Process signal through notifications handler
    process_via_notifications_handler(&inv_pool, &notif_pool, &tenant, item.id).await;
    let notif_count_1 = count_notif_outbox(&notif_pool, &tenant).await;
    assert_eq!(notif_count_1, 1, "one notifications outbox row created");

    // Issue 2 more — still below threshold (3 remaining); must NOT emit again (dedup)
    process_issue(
        &inv_pool,
        &IssueRequest {
            tenant_id: tenant.clone(),
            item_id: item.id,
            warehouse_id: wh,
            location_id: None,
            quantity: 2,
            currency: "usd".to_string(),
            source_module: "test".to_string(),
            source_type: "test_order".to_string(),
            source_id: "LS-ORD-002".to_string(),
            source_line_id: None,
            idempotency_key: key(),
            correlation_id: None,
            causation_id: None,
            uom_id: None,
            lot_code: None,
            serial_codes: None,
        },
        None,
    )
    .await
    .expect("second issue while below threshold");

    let count_2 = count_low_stock_outbox(&inv_pool, &tenant, item.id).await;
    assert_eq!(
        count_2, 1,
        "still below threshold → no additional signal (dedup)"
    );

    // Recovery: receipt brings qty above threshold AND creates FIFO layers
    // (adjustment would update item_on_hand but NOT create consumable FIFO layers,
    //  so a subsequent issue would fail with InsufficientQuantity from the layer guard).
    process_receipt(
        &inv_pool,
        &ReceiptRequest {
            tenant_id: tenant.clone(),
            item_id: item.id,
            warehouse_id: wh,
            location_id: None,
            quantity: 20,
            unit_cost_minor: 1_000,
            currency: "usd".to_string(),
            source_type: "purchase".to_string(),
            purchase_order_id: None,
            idempotency_key: key(),
            correlation_id: None,
            causation_id: None,
            lot_code: None,
            serial_codes: None,
            uom_id: None,
        },
        None,
    )
    .await
    .expect("recovery receipt — pushes qty to 23, re-arms reorder state");

    // Re-arm: issue again to drop below threshold
    process_issue(
        &inv_pool,
        &IssueRequest {
            tenant_id: tenant.clone(),
            item_id: item.id,
            warehouse_id: wh,
            location_id: None,
            quantity: 20,
            currency: "usd".to_string(),
            source_module: "test".to_string(),
            source_type: "test_order".to_string(),
            source_id: "LS-ORD-003".to_string(),
            source_line_id: None,
            idempotency_key: key(),
            correlation_id: None,
            causation_id: None,
            uom_id: None,
            lot_code: None,
            serial_codes: None,
        },
        None,
    )
    .await
    .expect("third issue — re-arm trigger");

    let count_3 = count_low_stock_outbox(&inv_pool, &tenant, item.id).await;
    assert_eq!(
        count_3, 2,
        "second threshold crossing after recovery → new signal (re-arm)"
    );

    cleanup(&inv_pool, &tenant).await;
    cleanup_notifications(&notif_pool, &tenant).await;
}

// ============================================================================
// Low-stock helpers
// ============================================================================

async fn count_low_stock_outbox(pool: &sqlx::PgPool, tenant_id: &str, item_id: Uuid) -> i64 {
    let (n,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM inv_outbox \
         WHERE tenant_id = $1 AND aggregate_id = $2::TEXT \
         AND event_type = 'inventory.low_stock_triggered'",
    )
    .bind(tenant_id)
    .bind(item_id.to_string())
    .fetch_one(pool)
    .await
    .expect("count low_stock outbox");
    n
}

async fn fetch_low_stock_envelope(
    pool: &sqlx::PgPool,
    tenant_id: &str,
    item_id: Uuid,
) -> serde_json::Value {
    let (v,): (serde_json::Value,) = sqlx::query_as(
        "SELECT payload FROM inv_outbox \
         WHERE tenant_id = $1 AND aggregate_id = $2::TEXT \
         AND event_type = 'inventory.low_stock_triggered' \
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(tenant_id)
    .bind(item_id.to_string())
    .fetch_one(pool)
    .await
    .expect("fetch low_stock envelope");
    v
}

async fn count_notif_outbox(pool: &sqlx::PgPool, tenant_id: &str) -> i64 {
    let (n,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM events_outbox \
         WHERE tenant_id = $1 AND subject = 'notifications.low_stock.alert.created'",
    )
    .bind(tenant_id)
    .fetch_one(pool)
    .await
    .expect("count notif outbox");
    n
}

async fn process_via_notifications_handler(
    inv_pool: &sqlx::PgPool,
    notif_pool: &sqlx::PgPool,
    tenant_id: &str,
    item_id: Uuid,
) {
    let envelope = fetch_low_stock_envelope(inv_pool, tenant_id, item_id).await;

    let event_id: Uuid = envelope
        .get("event_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
        .expect("event_id in envelope");

    let inner = envelope.get("payload").expect("payload in envelope");
    let payload: LowStockTriggeredPayload =
        serde_json::from_value(inner.clone()).expect("deserialize LowStockTriggeredPayload");

    let meta = EnvelopeMetadata {
        event_id,
        tenant_id: tenant_id.to_string(),
        correlation_id: None,
    };

    handle_low_stock_triggered(notif_pool, payload, meta)
        .await
        .expect("handle low_stock_triggered");
}

async fn cleanup_notifications(pool: &sqlx::PgPool, tenant_id: &str) {
    for q in [
        "DELETE FROM events_outbox WHERE tenant_id = $1",
        "DELETE FROM processed_events WHERE tenant_id = $1",
    ] {
        sqlx::query(q).bind(tenant_id).execute(pool).await.ok();
    }
}
