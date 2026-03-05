//! GL Inventory Consumer — source_type branching tests (bd-2vc9u)
//!
//! Tests the three GL posting paths:
//! 1. Purchase issue → COGS (DR COGS / CR INVENTORY)
//! 2. Production issue → WIP (DR WIP / CR INVENTORY)
//! 3. Production receipt → FG (DR INVENTORY / CR WIP)
//! Plus: idempotency holds across all paths.
//!
//! Real GL database, no mocks.

mod common;

use chrono::Utc;
use common::{cleanup_test_tenant, get_test_pool, setup_test_account, setup_test_period};
use gl_rs::consumers::gl_inventory_consumer::{
    process_inventory_cogs_posting, process_inventory_wip_posting,
    process_production_receipt_posting, ConsumedLayer, ItemIssuedPayload, ItemReceivedPayload,
    SourceRef,
};
use gl_rs::services::journal_service::JournalError;
use serial_test::serial;
use uuid::Uuid;

// ============================================================================
// Helpers
// ============================================================================

fn make_issue_payload(tenant_id: &str, source_type: &str) -> ItemIssuedPayload {
    let layer = ConsumedLayer {
        layer_id: Uuid::new_v4(),
        quantity: 10,
        unit_cost_minor: 5000,
        extended_cost_minor: 50_000,
    };
    ItemIssuedPayload {
        issue_line_id: Uuid::new_v4(),
        tenant_id: tenant_id.to_string(),
        item_id: Uuid::new_v4(),
        sku: "SKU-TEST-001".to_string(),
        warehouse_id: Uuid::new_v4(),
        quantity: 10,
        total_cost_minor: 50_000,
        currency: "usd".to_string(),
        consumed_layers: vec![layer],
        source_ref: SourceRef {
            source_module: "test".to_string(),
            source_type: source_type.to_string(),
            source_id: "REF-001".to_string(),
            source_line_id: None,
        },
        issued_at: Utc::now(),
    }
}

fn make_received_payload(tenant_id: &str, source_type: &str) -> ItemReceivedPayload {
    ItemReceivedPayload {
        receipt_line_id: Uuid::new_v4(),
        tenant_id: tenant_id.to_string(),
        item_id: Uuid::new_v4(),
        sku: "SKU-FG-001".to_string(),
        warehouse_id: Uuid::new_v4(),
        quantity: 5,
        unit_cost_minor: 20_000,
        currency: "usd".to_string(),
        source_type: source_type.to_string(),
        purchase_order_id: None,
        received_at: Utc::now(),
    }
}

async fn get_journal_lines(
    pool: &sqlx::PgPool,
    source_event_id: Uuid,
) -> Vec<(String, i64, i64)> {
    let entry_id: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM journal_entries WHERE source_event_id = $1 LIMIT 1")
            .bind(source_event_id)
            .fetch_optional(pool)
            .await
            .expect("query journal_entries");

    let entry_id = match entry_id {
        Some(id) => id,
        None => return vec![],
    };

    sqlx::query_as(
        r#"
        SELECT account_ref,
               COALESCE(debit_minor, 0) AS debit,
               COALESCE(credit_minor, 0) AS credit
        FROM journal_lines
        WHERE journal_entry_id = $1
        ORDER BY line_no
        "#,
    )
    .bind(entry_id)
    .fetch_all(pool)
    .await
    .expect("query journal_lines")
}

async fn cleanup_with_processed_events(pool: &sqlx::PgPool, tenant_id: &str) {
    sqlx::query(
        "DELETE FROM processed_events WHERE event_id IN (SELECT source_event_id FROM journal_entries WHERE tenant_id = $1)",
    )
    .bind(tenant_id)
    .execute(pool)
    .await
    .ok();
    cleanup_test_tenant(pool, tenant_id).await;
}

fn test_tenant() -> String {
    format!("test-gl-src-type-{}", Uuid::new_v4())
}

async fn setup_gl_accounts(pool: &sqlx::PgPool, tenant_id: &str) {
    setup_test_account(pool, tenant_id, "COGS", "Cost of Goods Sold", "expense", "debit").await;
    setup_test_account(pool, tenant_id, "INVENTORY", "Inventory Asset", "asset", "debit").await;
    setup_test_account(
        pool,
        tenant_id,
        "WIP",
        "Work in Process",
        "asset",
        "debit",
    )
    .await;

    setup_test_period(
        pool,
        tenant_id,
        chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        chrono::NaiveDate::from_ymd_opt(2026, 12, 31).unwrap(),
    )
    .await;
}

// ============================================================================
// Test 1: Purchase issue → COGS path (DR COGS / CR INVENTORY)
// ============================================================================

#[tokio::test]
#[serial]
async fn purchase_issue_posts_cogs_journal() {
    let pool = get_test_pool().await;
    let tenant_id = test_tenant();
    setup_gl_accounts(&pool, &tenant_id).await;

    let payload = make_issue_payload(&tenant_id, "purchase");
    let event_id = Uuid::new_v4();

    let entry_id =
        process_inventory_cogs_posting(&pool, event_id, &tenant_id, "inventory", &payload)
            .await
            .expect("COGS posting must succeed");
    assert_ne!(entry_id, Uuid::nil());

    let lines = get_journal_lines(&pool, event_id).await;
    assert_eq!(lines.len(), 2, "exactly 2 journal lines");

    let cogs = lines.iter().find(|(a, _, _)| a == "COGS").unwrap();
    let inv = lines.iter().find(|(a, _, _)| a == "INVENTORY").unwrap();

    assert_eq!(cogs.1, 50_000, "COGS debit = $500.00");
    assert_eq!(cogs.2, 0, "COGS credit = 0");
    assert_eq!(inv.1, 0, "INVENTORY debit = 0");
    assert_eq!(inv.2, 50_000, "INVENTORY credit = $500.00");
    assert_eq!(cogs.1 + inv.1, cogs.2 + inv.2, "balanced");

    cleanup_with_processed_events(&pool, &tenant_id).await;
    pool.close().await;
}

// ============================================================================
// Test 2: Production issue → WIP path (DR WIP / CR INVENTORY)
// ============================================================================

#[tokio::test]
#[serial]
async fn production_issue_posts_wip_journal() {
    let pool = get_test_pool().await;
    let tenant_id = test_tenant();
    setup_gl_accounts(&pool, &tenant_id).await;

    let payload = make_issue_payload(&tenant_id, "production");
    let event_id = Uuid::new_v4();

    let entry_id =
        process_inventory_wip_posting(&pool, event_id, &tenant_id, "inventory", &payload)
            .await
            .expect("WIP posting must succeed");
    assert_ne!(entry_id, Uuid::nil());

    let lines = get_journal_lines(&pool, event_id).await;
    assert_eq!(lines.len(), 2, "exactly 2 journal lines");

    let wip = lines.iter().find(|(a, _, _)| a == "WIP").unwrap();
    let inv = lines.iter().find(|(a, _, _)| a == "INVENTORY").unwrap();

    assert_eq!(wip.1, 50_000, "WIP debit = $500.00");
    assert_eq!(wip.2, 0, "WIP credit = 0");
    assert_eq!(inv.1, 0, "INVENTORY debit = 0");
    assert_eq!(inv.2, 50_000, "INVENTORY credit = $500.00");
    assert_eq!(wip.1 + inv.1, wip.2 + inv.2, "balanced");

    cleanup_with_processed_events(&pool, &tenant_id).await;
    pool.close().await;
}

// ============================================================================
// Test 3: Production receipt → FG recognition (DR INVENTORY / CR WIP)
// ============================================================================

#[tokio::test]
#[serial]
async fn production_receipt_posts_fg_journal() {
    let pool = get_test_pool().await;
    let tenant_id = test_tenant();
    setup_gl_accounts(&pool, &tenant_id).await;

    let payload = make_received_payload(&tenant_id, "production");
    let event_id = Uuid::new_v4();

    // 5 units x $200.00 = $1000.00 = 100_000 minor
    let entry_id =
        process_production_receipt_posting(&pool, event_id, &tenant_id, "inventory", &payload)
            .await
            .expect("Production receipt posting must succeed");
    assert_ne!(entry_id, Uuid::nil());

    let lines = get_journal_lines(&pool, event_id).await;
    assert_eq!(lines.len(), 2, "exactly 2 journal lines");

    let inv = lines.iter().find(|(a, _, _)| a == "INVENTORY").unwrap();
    let wip = lines.iter().find(|(a, _, _)| a == "WIP").unwrap();

    assert_eq!(inv.1, 100_000, "INVENTORY debit = $1000.00");
    assert_eq!(inv.2, 0, "INVENTORY credit = 0");
    assert_eq!(wip.1, 0, "WIP debit = 0");
    assert_eq!(wip.2, 100_000, "WIP credit = $1000.00");
    assert_eq!(inv.1 + wip.1, inv.2 + wip.2, "balanced");

    cleanup_with_processed_events(&pool, &tenant_id).await;
    pool.close().await;
}

// ============================================================================
// Test 4: WIP posting idempotency — duplicate event_id returns DuplicateEvent
// ============================================================================

#[tokio::test]
#[serial]
async fn wip_posting_is_idempotent() {
    let pool = get_test_pool().await;
    let tenant_id = test_tenant();
    setup_gl_accounts(&pool, &tenant_id).await;

    let payload = make_issue_payload(&tenant_id, "production");
    let event_id = Uuid::new_v4();

    process_inventory_wip_posting(&pool, event_id, &tenant_id, "inventory", &payload)
        .await
        .expect("first WIP posting must succeed");

    let second =
        process_inventory_wip_posting(&pool, event_id, &tenant_id, "inventory", &payload).await;

    assert!(
        matches!(second, Err(JournalError::DuplicateEvent(_))),
        "second post must return DuplicateEvent, got: {:?}",
        second
    );

    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM journal_entries WHERE source_event_id = $1")
            .bind(event_id)
            .fetch_one(&pool)
            .await
            .expect("count query");
    assert_eq!(count, 1, "no duplicate journal entries");

    cleanup_with_processed_events(&pool, &tenant_id).await;
    pool.close().await;
}

// ============================================================================
// Test 5: Production receipt idempotency
// ============================================================================

#[tokio::test]
#[serial]
async fn production_receipt_posting_is_idempotent() {
    let pool = get_test_pool().await;
    let tenant_id = test_tenant();
    setup_gl_accounts(&pool, &tenant_id).await;

    let payload = make_received_payload(&tenant_id, "production");
    let event_id = Uuid::new_v4();

    process_production_receipt_posting(&pool, event_id, &tenant_id, "inventory", &payload)
        .await
        .expect("first production receipt posting must succeed");

    let second =
        process_production_receipt_posting(&pool, event_id, &tenant_id, "inventory", &payload)
            .await;

    assert!(
        matches!(second, Err(JournalError::DuplicateEvent(_))),
        "second post must return DuplicateEvent, got: {:?}",
        second
    );

    cleanup_with_processed_events(&pool, &tenant_id).await;
    pool.close().await;
}
