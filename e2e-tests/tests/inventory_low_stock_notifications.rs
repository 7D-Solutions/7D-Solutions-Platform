//! E2E Test: Low Stock Signal → Notifications Persistence (bd-3519)
//!
//! ## Coverage
//! 1. Issue stock below reorder_point → `inventory.low_stock_triggered` appears in inv_outbox.
//! 2. Notifications handler processes the event → `events_outbox` row persisted.
//! 3. While still below threshold: a second issue does NOT emit another low_stock signal (dedup).
//! 4. Recovery adjustment drives qty above threshold → resets below_threshold state.
//! 5. Second below-threshold crossing → new signal emitted (re-arm confirmed).
//!
//! ## Pattern
//! No Docker, no mocks — real inventory DB (port 5442) and real notifications DB (port 5437).
//! Handler functions are called directly; no NATS required.

mod common;

use anyhow::Result;
use common::generate_test_tenant;
use inventory_rs::domain::{
    adjust_service::{process_adjustment, AdjustRequest},
    issue_service::{process_issue, IssueRequest},
    items::{CreateItemRequest, ItemRepo, TrackingMode},
    receipt_service::{process_receipt, ReceiptRequest},
    reorder::models::{CreateReorderPolicyRequest, ReorderPolicyRepo},
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
// Setup helpers
// ============================================================================

/// Seed an item + warehouse + reorder policy + initial receipt.
/// Returns (item_id, warehouse_id).
async fn setup_inventory(
    pool: &sqlx::PgPool,
    tenant_id: &str,
    reorder_point: i64,
    initial_qty: i64,
) -> Result<(Uuid, Uuid)> {
    let item = ItemRepo::create(
        pool,
        &CreateItemRequest {
            tenant_id: tenant_id.to_string(),
            sku: format!("LSN-{}", Uuid::new_v4()),
            name: "Low Stock Notification Test Item".to_string(),
            description: None,
            inventory_account_ref: "1200".to_string(),
            cogs_account_ref: "5000".to_string(),
            variance_account_ref: "5010".to_string(),
            uom: None,
            tracking_mode: TrackingMode::None,
            make_buy: None,
        },
    )
    .await?;

    let warehouse_id = Uuid::new_v4();

    // Create a global (null-location) reorder policy for this item
    ReorderPolicyRepo::create(
        pool,
        &CreateReorderPolicyRequest {
            tenant_id: tenant_id.to_string(),
            item_id: item.id,
            location_id: None,
            reorder_point,
            safety_stock: 0,
            max_qty: None,
            notes: Some("E2E test policy".to_string()),
            created_by: Some("e2e-test".to_string()),
        },
    )
    .await?;

    // Receipt: bring stock up to initial_qty
    process_receipt(
        pool,
        &ReceiptRequest {
            tenant_id: tenant_id.to_string(),
            item_id: item.id,
            warehouse_id,
            location_id: None,
            quantity: initial_qty,
            unit_cost_minor: 1_000,
            currency: "USD".to_string(),
            source_type: "purchase".to_string(),
            purchase_order_id: None,
            lot_code: None,
            serial_codes: None,
            uom_id: None,
            idempotency_key: format!("rcpt-setup-{}", Uuid::new_v4()),
            correlation_id: Some("e2e-setup".to_string()),
            causation_id: None,
        },
        None,
    )
    .await?;

    Ok((item.id, warehouse_id))
}

// ============================================================================
// Query helpers
// ============================================================================

/// Count low_stock_triggered events in inv_outbox for a given (tenant, item).
async fn count_low_stock_outbox(
    pool: &sqlx::PgPool,
    tenant_id: &str,
    item_id: Uuid,
) -> Result<i64> {
    let row: (i64,) = sqlx::query_as(
        r#"
        SELECT COUNT(*) FROM inv_outbox
        WHERE tenant_id = $1
          AND aggregate_id = $2::TEXT
          AND event_type = 'inventory.low_stock_triggered'
        "#,
    )
    .bind(tenant_id)
    .bind(item_id.to_string())
    .fetch_one(pool)
    .await?;
    Ok(row.0)
}

/// Fetch the most-recent low_stock outbox envelope for a given (tenant, item).
async fn fetch_low_stock_envelope(
    pool: &sqlx::PgPool,
    tenant_id: &str,
    item_id: Uuid,
) -> Result<serde_json::Value> {
    let row: (serde_json::Value,) = sqlx::query_as(
        r#"
        SELECT payload FROM inv_outbox
        WHERE tenant_id = $1
          AND aggregate_id = $2::TEXT
          AND event_type = 'inventory.low_stock_triggered'
        ORDER BY created_at DESC
        LIMIT 1
        "#,
    )
    .bind(tenant_id)
    .bind(item_id.to_string())
    .fetch_one(pool)
    .await?;
    Ok(row.0)
}

/// Count `notifications.low_stock.alert.created` rows in notifications events_outbox.
async fn count_notif_outbox(pool: &sqlx::PgPool, tenant_id: &str) -> Result<i64> {
    let row: (i64,) = sqlx::query_as(
        r#"
        SELECT COUNT(*) FROM events_outbox
        WHERE tenant_id = $1
          AND subject = 'notifications.low_stock.alert.created'
        "#,
    )
    .bind(tenant_id)
    .fetch_one(pool)
    .await?;
    Ok(row.0)
}

// ============================================================================
// Cleanup
// ============================================================================

async fn cleanup_inventory(pool: &sqlx::PgPool, tenant_id: &str) {
    for q in [
        "DELETE FROM inv_outbox WHERE tenant_id = $1",
        "DELETE FROM inv_idempotency_keys WHERE tenant_id = $1",
        "DELETE FROM reorder_signal_state WHERE tenant_id = $1",
        "DELETE FROM reorder_policies WHERE tenant_id = $1",
        "DELETE FROM layer_consumptions WHERE ledger_entry_id IN (SELECT id FROM inventory_ledger WHERE tenant_id = $1)",
        "DELETE FROM item_on_hand WHERE tenant_id = $1",
        "DELETE FROM inv_adjustments WHERE tenant_id = $1",
        "DELETE FROM inventory_layers WHERE tenant_id = $1",
        "DELETE FROM inventory_ledger WHERE tenant_id = $1",
        "DELETE FROM items WHERE tenant_id = $1",
    ] {
        sqlx::query(q).bind(tenant_id).execute(pool).await.ok();
    }
}

async fn cleanup_notifications(pool: &sqlx::PgPool, tenant_id: &str) {
    for q in [
        "DELETE FROM events_outbox WHERE tenant_id = $1",
        "DELETE FROM processed_events WHERE tenant_id = $1",
    ] {
        sqlx::query(q).bind(tenant_id).execute(pool).await.ok();
    }
}

// ============================================================================
// Helper: parse outbox envelope → notifications handler call
// ============================================================================

/// Parse a low_stock_triggered envelope from inv_outbox and call the notifications handler.
async fn process_via_notifications_handler(
    inv_pool: &sqlx::PgPool,
    notif_pool: &sqlx::PgPool,
    tenant_id: &str,
    item_id: Uuid,
) -> Result<()> {
    let envelope = fetch_low_stock_envelope(inv_pool, tenant_id, item_id).await?;

    let event_id: Uuid = envelope
        .get("event_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or_else(|| anyhow::anyhow!("Missing event_id in envelope"))?;

    let inner = envelope
        .get("payload")
        .ok_or_else(|| anyhow::anyhow!("Missing payload in envelope"))?;
    let payload: LowStockTriggeredPayload = serde_json::from_value(inner.clone())?;

    let metadata = EnvelopeMetadata {
        event_id,
        tenant_id: tenant_id.to_string(),
        correlation_id: envelope
            .get("correlation_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
    };

    handle_low_stock_triggered(notif_pool, payload, metadata)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(())
}

// ============================================================================
// Test 1: Basic low-stock signal + notifications persistence
// ============================================================================

/// Issue below reorder_point → inv_outbox gets low_stock_triggered.
/// Notifications handler persists the alert to events_outbox.
#[tokio::test]
#[serial]
async fn low_stock_signal_emitted_and_notification_persisted() {
    let inv_pool = get_inventory_pool().await;
    let notif_pool = get_notifications_pool().await;
    let tenant_id = generate_test_tenant();

    // Arrange: 100 units, reorder_point=50
    let (item_id, warehouse_id) = setup_inventory(&inv_pool, &tenant_id, 50, 100)
        .await
        .expect("setup_inventory");

    // Act: issue 60 units → available drops to 40, which is < 50
    process_issue(
        &inv_pool,
        &IssueRequest {
            tenant_id: tenant_id.clone(),
            item_id,
            warehouse_id,
            location_id: None,
            quantity: 60,
            currency: "USD".to_string(),
            source_module: "e2e-test".to_string(),
            source_type: "test".to_string(),
            source_id: Uuid::new_v4().to_string(),
            source_line_id: None,
            idempotency_key: format!("issue-1-{}", Uuid::new_v4()),
            correlation_id: Some("e2e-corr-1".to_string()),
            causation_id: None,
            uom_id: None,
            lot_code: None,
            serial_codes: None,
        },
        None,
    )
    .await
    .expect("process_issue");

    // Assert: inv_outbox has exactly 1 low_stock_triggered event
    let outbox_count = count_low_stock_outbox(&inv_pool, &tenant_id, item_id)
        .await
        .expect("count_low_stock_outbox");
    assert_eq!(
        outbox_count, 1,
        "Expected 1 low_stock_triggered in inv_outbox, got {outbox_count}"
    );

    // Act: call notifications handler directly (simulates NATS consumer)
    process_via_notifications_handler(&inv_pool, &notif_pool, &tenant_id, item_id)
        .await
        .expect("process_via_notifications_handler");

    // Assert: notifications events_outbox has the alert
    let notif_count = count_notif_outbox(&notif_pool, &tenant_id)
        .await
        .expect("count_notif_outbox");
    assert_eq!(
        notif_count, 1,
        "Expected 1 alert in notifications events_outbox, got {notif_count}"
    );

    cleanup_inventory(&inv_pool, &tenant_id).await;
    cleanup_notifications(&notif_pool, &tenant_id).await;
}

// ============================================================================
// Test 2: Dedup — no duplicate signal while still below threshold
// ============================================================================

/// A second issue while still below threshold must NOT emit another low_stock signal.
#[tokio::test]
#[serial]
async fn low_stock_dedup_no_second_signal_while_below_threshold() {
    let inv_pool = get_inventory_pool().await;
    let tenant_id = generate_test_tenant();

    // 100 units, reorder_point=50
    let (item_id, warehouse_id) = setup_inventory(&inv_pool, &tenant_id, 50, 100)
        .await
        .expect("setup_inventory");

    // Issue 1: 60 → qty=40, below threshold → emits signal
    process_issue(
        &inv_pool,
        &IssueRequest {
            tenant_id: tenant_id.clone(),
            item_id,
            warehouse_id,
            location_id: None,
            quantity: 60,
            currency: "USD".to_string(),
            source_module: "e2e-test".to_string(),
            source_type: "test".to_string(),
            source_id: Uuid::new_v4().to_string(),
            source_line_id: None,
            idempotency_key: format!("issue-dedup-1-{}", Uuid::new_v4()),
            correlation_id: Some("e2e-dedup-1".to_string()),
            causation_id: None,
            uom_id: None,
            lot_code: None,
            serial_codes: None,
        },
        None,
    )
    .await
    .expect("process_issue #1");

    let after_first = count_low_stock_outbox(&inv_pool, &tenant_id, item_id)
        .await
        .expect("count after first issue");
    assert_eq!(after_first, 1, "First issue should emit exactly 1 signal");

    // Issue 2: 5 more → qty=35, still below threshold → must NOT emit a second signal
    process_issue(
        &inv_pool,
        &IssueRequest {
            tenant_id: tenant_id.clone(),
            item_id,
            warehouse_id,
            location_id: None,
            quantity: 5,
            currency: "USD".to_string(),
            source_module: "e2e-test".to_string(),
            source_type: "test".to_string(),
            source_id: Uuid::new_v4().to_string(),
            source_line_id: None,
            idempotency_key: format!("issue-dedup-2-{}", Uuid::new_v4()),
            correlation_id: Some("e2e-dedup-2".to_string()),
            causation_id: None,
            uom_id: None,
            lot_code: None,
            serial_codes: None,
        },
        None,
    )
    .await
    .expect("process_issue #2");

    let after_second = count_low_stock_outbox(&inv_pool, &tenant_id, item_id)
        .await
        .expect("count after second issue");
    assert_eq!(
        after_second, 1,
        "Second issue while still below threshold must not emit a duplicate signal (got {after_second})"
    );

    cleanup_inventory(&inv_pool, &tenant_id).await;
}

// ============================================================================
// Test 3: Recovery re-arms the signal
// ============================================================================

/// After recovering above threshold, the next below-threshold crossing should
/// emit a new signal (state is re-armed).
#[tokio::test]
#[serial]
async fn low_stock_rearm_after_recovery() {
    let inv_pool = get_inventory_pool().await;
    let notif_pool = get_notifications_pool().await;
    let tenant_id = generate_test_tenant();

    // 100 units, reorder_point=50
    let (item_id, warehouse_id) = setup_inventory(&inv_pool, &tenant_id, 50, 100)
        .await
        .expect("setup_inventory");

    // Crossing #1: issue 60 → qty=40, below threshold → signal #1
    process_issue(
        &inv_pool,
        &IssueRequest {
            tenant_id: tenant_id.clone(),
            item_id,
            warehouse_id,
            location_id: None,
            quantity: 60,
            currency: "USD".to_string(),
            source_module: "e2e-test".to_string(),
            source_type: "test".to_string(),
            source_id: Uuid::new_v4().to_string(),
            source_line_id: None,
            idempotency_key: format!("issue-rearm-1-{}", Uuid::new_v4()),
            correlation_id: Some("e2e-rearm-1".to_string()),
            causation_id: None,
            uom_id: None,
            lot_code: None,
            serial_codes: None,
        },
        None,
    )
    .await
    .expect("process_issue crossing #1");

    let after_crossing1 = count_low_stock_outbox(&inv_pool, &tenant_id, item_id)
        .await
        .unwrap();
    assert_eq!(after_crossing1, 1, "First crossing should produce 1 signal");

    // Recovery: adjustment +20 → qty=60, above threshold → state resets (no new signal)
    process_adjustment(
        &inv_pool,
        &AdjustRequest {
            tenant_id: tenant_id.clone(),
            item_id,
            warehouse_id,
            location_id: None,
            quantity_delta: 20,
            reason: "restock".to_string(),
            allow_negative: false,
            idempotency_key: format!("adj-recovery-{}", Uuid::new_v4()),
            correlation_id: Some("e2e-recovery".to_string()),
            causation_id: None,
        },
        None,
    )
    .await
    .expect("recovery adjustment");

    // No new low_stock signal during recovery (going UP does not trigger)
    let after_recovery = count_low_stock_outbox(&inv_pool, &tenant_id, item_id)
        .await
        .unwrap();
    assert_eq!(
        after_recovery, 1,
        "Recovery should not add a new low_stock signal"
    );

    // Crossing #2: issue 25 → qty=35, below threshold again → signal #2
    process_issue(
        &inv_pool,
        &IssueRequest {
            tenant_id: tenant_id.clone(),
            item_id,
            warehouse_id,
            location_id: None,
            quantity: 25,
            currency: "USD".to_string(),
            source_module: "e2e-test".to_string(),
            source_type: "test".to_string(),
            source_id: Uuid::new_v4().to_string(),
            source_line_id: None,
            idempotency_key: format!("issue-rearm-2-{}", Uuid::new_v4()),
            correlation_id: Some("e2e-rearm-2".to_string()),
            causation_id: None,
            uom_id: None,
            lot_code: None,
            serial_codes: None,
        },
        None,
    )
    .await
    .expect("process_issue crossing #2");

    let after_crossing2 = count_low_stock_outbox(&inv_pool, &tenant_id, item_id)
        .await
        .unwrap();
    assert_eq!(
        after_crossing2, 2,
        "Second crossing after recovery should emit a new signal (total=2, got {after_crossing2})"
    );

    // Drive signal #2 through notifications handler → 1 new alert in notifications outbox
    process_via_notifications_handler(&inv_pool, &notif_pool, &tenant_id, item_id)
        .await
        .expect("handle second notification");

    let notif_count = count_notif_outbox(&notif_pool, &tenant_id).await.unwrap();
    assert_eq!(
        notif_count, 1,
        "After second crossing, notifications outbox should have 1 alert (got {notif_count})"
    );

    cleanup_inventory(&inv_pool, &tenant_id).await;
    cleanup_notifications(&notif_pool, &tenant_id).await;
}
