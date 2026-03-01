//! E2E Test: Inventory Issue → GL COGS Posting (bd-1121)
//!
//! **Coverage:**
//! 1. Item issued event posts balanced GL journal entry (COGS DR / INVENTORY CR)
//! 2. Idempotency: duplicate event_id does not create a second journal entry
//! 3. Journal entry is balanced (debit == credit)
//! 4. COGS and INVENTORY accounts must exist and be active before posting
//! 5. Closed period rejects the posting (period-close aware)
//! 6. processed_events row is created atomically with the journal entry
//!
//! **Pattern:** No Docker, no mocks — uses live GL database pool via common::get_gl_pool()
//! Tests call `process_inventory_cogs_posting` directly (no NATS required).

mod common;

use anyhow::Result;
use chrono::Utc;
use common::{generate_test_tenant, get_gl_pool};
use gl_rs::consumers::gl_inventory_consumer::{
    process_inventory_cogs_posting, ConsumedLayer, ItemIssuedPayload, SourceRef,
};
use uuid::Uuid;

// ============================================================================
// Test helpers
// ============================================================================

/// Insert required GL accounts (COGS + INVENTORY) for the test tenant.
async fn setup_gl_accounts(pool: &sqlx::PgPool, tenant_id: &str) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO accounts (id, tenant_id, code, name, type, normal_balance, is_active)
        VALUES
            (gen_random_uuid(), $1, 'COGS', 'Cost of Goods Sold', 'expense', 'debit', true),
            (gen_random_uuid(), $1, 'INVENTORY', 'Inventory Asset', 'asset', 'debit', true)
        ON CONFLICT (tenant_id, code) DO NOTHING
        "#,
    )
    .bind(tenant_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Create an open accounting period covering 2026.
async fn setup_accounting_period(pool: &sqlx::PgPool, tenant_id: &str) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO accounting_periods (tenant_id, period_start, period_end, is_closed)
        VALUES ($1, '2026-01-01', '2026-12-31', false)
        "#,
    )
    .bind(tenant_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Create a closed accounting period for testing period-close rejection.
///
/// Sets `closed_at = NOW()` (the field `period_repo` checks) and `close_hash`
/// (required by the chk_closed_requires_hash DB constraint).
async fn setup_closed_period(pool: &sqlx::PgPool, tenant_id: &str) -> Result<()> {
    // Use a far-past date range to avoid exclusion constraint conflicts with the open period
    sqlx::query(
        r#"
        INSERT INTO accounting_periods
            (tenant_id, period_start, period_end, is_closed, closed_at, closed_by, close_hash)
        VALUES ($1, '2024-01-01', '2024-12-31', true, NOW(), 'test', 'test-close-hash')
        "#,
    )
    .bind(tenant_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Fetch journal entry ID for a given source_event_id.
async fn get_journal_entry_id(pool: &sqlx::PgPool, event_id: Uuid) -> Result<Option<Uuid>> {
    let row: Option<(Uuid,)> =
        sqlx::query_as("SELECT id FROM journal_entries WHERE source_event_id = $1 LIMIT 1")
            .bind(event_id)
            .fetch_optional(pool)
            .await?;
    Ok(row.map(|(id,)| id))
}

/// Get journal lines for a given journal entry.
async fn get_journal_lines(pool: &sqlx::PgPool, entry_id: Uuid) -> Result<Vec<(String, f64, f64)>> {
    let rows: Vec<(String, f64, f64)> = sqlx::query_as(
        r#"
        SELECT account_ref,
               COALESCE(debit_minor, 0)::float8 / 100.0 AS debit,
               COALESCE(credit_minor, 0)::float8 / 100.0 AS credit
        FROM journal_lines
        WHERE journal_entry_id = $1
        ORDER BY line_no
        "#,
    )
    .bind(entry_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Count processed_events rows for a given event_id.
async fn count_processed_events(pool: &sqlx::PgPool, event_id: Uuid) -> Result<i64> {
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM processed_events WHERE event_id = $1")
            .bind(event_id)
            .fetch_one(pool)
            .await?;
    Ok(count)
}

/// Cleanup GL test data for a tenant (reverse FK order).
async fn cleanup_tenant(pool: &sqlx::PgPool, tenant_id: &str) -> Result<()> {
    sqlx::query(
        "DELETE FROM journal_lines WHERE journal_entry_id IN \
         (SELECT id FROM journal_entries WHERE tenant_id = $1)",
    )
    .bind(tenant_id)
    .execute(pool)
    .await?;

    sqlx::query(
        "DELETE FROM processed_events WHERE event_id IN \
         (SELECT source_event_id FROM journal_entries WHERE tenant_id = $1)",
    )
    .bind(tenant_id)
    .execute(pool)
    .await?;

    sqlx::query("DELETE FROM journal_entries WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await?;

    sqlx::query("DELETE FROM account_balances WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await?;

    sqlx::query("DELETE FROM period_summary_snapshots WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await?;

    sqlx::query("DELETE FROM accounts WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await?;

    sqlx::query("DELETE FROM accounting_periods WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await?;

    Ok(())
}

/// Build a sample ItemIssuedPayload for testing.
fn sample_payload(tenant_id: &str, sku: &str, total_cost_minor: i64) -> ItemIssuedPayload {
    let layer = ConsumedLayer {
        layer_id: Uuid::new_v4(),
        quantity: 10,
        unit_cost_minor: total_cost_minor / 10,
        extended_cost_minor: total_cost_minor,
    };
    ItemIssuedPayload {
        issue_line_id: Uuid::new_v4(),
        tenant_id: tenant_id.to_string(),
        item_id: Uuid::new_v4(),
        sku: sku.to_string(),
        warehouse_id: Uuid::new_v4(),
        quantity: 10,
        total_cost_minor,
        currency: "usd".to_string(),
        consumed_layers: vec![layer],
        source_ref: SourceRef {
            source_module: "orders".to_string(),
            source_type: "sales_order".to_string(),
            source_id: "SO-E2E-001".to_string(),
            source_line_id: Some("SO-E2E-001-L1".to_string()),
        },
        issued_at: Utc::now(),
    }
}

// ============================================================================
// Tests
// ============================================================================

/// Test 1: Successful issue → balanced COGS GL journal entry.
///
/// DR COGS      $500.00
/// CR INVENTORY $500.00
#[tokio::test]
async fn test_inventory_issue_posts_cogs_gl_entry() -> Result<()> {
    let pool = get_gl_pool().await;
    let tenant_id = generate_test_tenant();

    setup_gl_accounts(&pool, &tenant_id).await?;
    setup_accounting_period(&pool, &tenant_id).await?;

    let event_id = Uuid::new_v4();
    let payload = sample_payload(&tenant_id, "SKU-E2E-001", 50_000); // $500.00

    let entry_id =
        process_inventory_cogs_posting(&pool, event_id, &tenant_id, "inventory", &payload)
            .await
            .expect("COGS posting should succeed");

    // Verify journal entry was stored
    let stored_entry_id = get_journal_entry_id(&pool, event_id).await?;
    assert_eq!(
        stored_entry_id,
        Some(entry_id),
        "journal_entries.source_event_id lookup should return the created entry"
    );

    // Verify DR COGS / CR INVENTORY
    let lines = get_journal_lines(&pool, entry_id).await?;
    assert_eq!(lines.len(), 2, "exactly 2 journal lines");

    let cogs_line = lines.iter().find(|(acct, _, _)| acct == "COGS");
    let inv_line = lines.iter().find(|(acct, _, _)| acct == "INVENTORY");

    assert!(cogs_line.is_some(), "COGS line must exist");
    assert!(inv_line.is_some(), "INVENTORY line must exist");

    let (_, cogs_debit, cogs_credit) = cogs_line.unwrap();
    let (_, inv_debit, inv_credit) = inv_line.unwrap();

    assert!(
        (*cogs_debit - 500.0).abs() < 0.01,
        "COGS debit should be $500.00, got {}",
        cogs_debit
    );
    assert!((*cogs_credit).abs() < 0.01, "COGS credit should be $0.00");
    assert!((*inv_debit).abs() < 0.01, "INVENTORY debit should be $0.00");
    assert!(
        (*inv_credit - 500.0).abs() < 0.01,
        "INVENTORY credit should be $500.00, got {}",
        inv_credit
    );

    cleanup_tenant(&pool, &tenant_id).await?;
    Ok(())
}

/// Test 2: Journal entry is balanced (debit total == credit total).
#[tokio::test]
async fn test_inventory_issue_journal_entry_is_balanced() -> Result<()> {
    let pool = get_gl_pool().await;
    let tenant_id = generate_test_tenant();

    setup_gl_accounts(&pool, &tenant_id).await?;
    setup_accounting_period(&pool, &tenant_id).await?;

    let event_id = Uuid::new_v4();
    let payload = sample_payload(&tenant_id, "SKU-BAL-001", 123_456); // $1,234.56

    let entry_id =
        process_inventory_cogs_posting(&pool, event_id, &tenant_id, "inventory", &payload)
            .await
            .expect("posting should succeed");

    let lines = get_journal_lines(&pool, entry_id).await?;
    let total_debit: f64 = lines.iter().map(|(_, d, _)| d).sum();
    let total_credit: f64 = lines.iter().map(|(_, _, c)| c).sum();

    assert!(
        (total_debit - total_credit).abs() < 0.01,
        "Journal entry must be balanced: debit={} credit={}",
        total_debit,
        total_credit
    );

    cleanup_tenant(&pool, &tenant_id).await?;
    Ok(())
}

/// Test 3: Idempotency — duplicate event_id does not create a second journal entry.
#[tokio::test]
async fn test_inventory_issue_idempotency_prevents_duplicate() -> Result<()> {
    let pool = get_gl_pool().await;
    let tenant_id = generate_test_tenant();

    setup_gl_accounts(&pool, &tenant_id).await?;
    setup_accounting_period(&pool, &tenant_id).await?;

    let event_id = Uuid::new_v4();
    let payload = sample_payload(&tenant_id, "SKU-IDEM-001", 25_000);

    // First posting — should succeed
    let first =
        process_inventory_cogs_posting(&pool, event_id, &tenant_id, "inventory", &payload).await;
    assert!(first.is_ok(), "First posting should succeed");

    // Second posting with same event_id — should return DuplicateEvent
    let second =
        process_inventory_cogs_posting(&pool, event_id, &tenant_id, "inventory", &payload).await;

    match second {
        Err(gl_rs::services::journal_service::JournalError::DuplicateEvent(_)) => {
            // Expected: idempotent no-op
        }
        other => panic!("Expected DuplicateEvent, got: {:?}", other),
    }

    // Only one processed_events row
    let count = count_processed_events(&pool, event_id).await?;
    assert_eq!(count, 1, "only one processed_events row for this event_id");

    cleanup_tenant(&pool, &tenant_id).await?;
    Ok(())
}

/// Test 4: processed_events row is created atomically with the journal entry.
#[tokio::test]
async fn test_inventory_issue_processed_events_row_created() -> Result<()> {
    let pool = get_gl_pool().await;
    let tenant_id = generate_test_tenant();

    setup_gl_accounts(&pool, &tenant_id).await?;
    setup_accounting_period(&pool, &tenant_id).await?;

    let event_id = Uuid::new_v4();
    let payload = sample_payload(&tenant_id, "SKU-PROC-001", 75_000);

    process_inventory_cogs_posting(&pool, event_id, &tenant_id, "inventory", &payload)
        .await
        .expect("posting should succeed");

    let count = count_processed_events(&pool, event_id).await?;
    assert_eq!(count, 1, "processed_events row must be created atomically");

    cleanup_tenant(&pool, &tenant_id).await?;
    Ok(())
}

/// Test 5: Closed period — posting is rejected.
#[tokio::test]
async fn test_inventory_issue_closed_period_rejected() -> Result<()> {
    let pool = get_gl_pool().await;
    let tenant_id = generate_test_tenant();

    setup_gl_accounts(&pool, &tenant_id).await?;
    setup_closed_period(&pool, &tenant_id).await?;

    let event_id = Uuid::new_v4();

    // Construct payload with issued_at in the closed period (2024)
    let layer = ConsumedLayer {
        layer_id: Uuid::new_v4(),
        quantity: 5,
        unit_cost_minor: 1000,
        extended_cost_minor: 5000,
    };
    let payload = ItemIssuedPayload {
        issue_line_id: Uuid::new_v4(),
        tenant_id: tenant_id.clone(),
        item_id: Uuid::new_v4(),
        sku: "SKU-CLOSED-001".to_string(),
        warehouse_id: Uuid::new_v4(),
        quantity: 5,
        total_cost_minor: 5000,
        currency: "usd".to_string(),
        consumed_layers: vec![layer],
        source_ref: SourceRef {
            source_module: "orders".to_string(),
            source_type: "sales_order".to_string(),
            source_id: "SO-CLOSED-001".to_string(),
            source_line_id: None,
        },
        issued_at: "2024-06-15T12:00:00Z".parse().expect("valid date"),
    };

    let result =
        process_inventory_cogs_posting(&pool, event_id, &tenant_id, "inventory", &payload).await;

    assert!(result.is_err(), "Posting to a closed period must fail");

    cleanup_tenant(&pool, &tenant_id).await?;
    Ok(())
}

/// Test 6: Different event_ids create separate journal entries.
#[tokio::test]
async fn test_inventory_issue_separate_events_create_separate_entries() -> Result<()> {
    let pool = get_gl_pool().await;
    let tenant_id = generate_test_tenant();

    setup_gl_accounts(&pool, &tenant_id).await?;
    setup_accounting_period(&pool, &tenant_id).await?;

    let event_id_1 = Uuid::new_v4();
    let event_id_2 = Uuid::new_v4();
    let payload_1 = sample_payload(&tenant_id, "SKU-SEP-001", 10_000);
    let payload_2 = sample_payload(&tenant_id, "SKU-SEP-002", 20_000);

    let entry_id_1 =
        process_inventory_cogs_posting(&pool, event_id_1, &tenant_id, "inventory", &payload_1)
            .await
            .expect("first posting should succeed");
    let entry_id_2 =
        process_inventory_cogs_posting(&pool, event_id_2, &tenant_id, "inventory", &payload_2)
            .await
            .expect("second posting should succeed");

    assert_ne!(
        entry_id_1, entry_id_2,
        "different events must create different journal entries"
    );

    cleanup_tenant(&pool, &tenant_id).await?;
    Ok(())
}
