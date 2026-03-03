//! GL Export integration tests (real Postgres on port 5438)
//!
//! Covers all 6 required test categories:
//! 1. QuickBooks export E2E
//! 2. Xero export E2E
//! 3. Tenant isolation
//! 4. Idempotency
//! 5. Outbox event verification
//! 6. Empty data export

mod common;

use common::get_test_pool;
use gl_rs::exports::service::{
    execute_export, ExportFormat, ExportRequest, ExportType, EVENT_TYPE_EXPORT_COMPLETED,
};
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

/// Setup accounts and journal entries for export testing
async fn setup_export_data(pool: &PgPool, tenant_id: &str) -> Uuid {
    // Create accounts
    sqlx::query(
        r#"
        INSERT INTO accounts (id, tenant_id, code, name, type, normal_balance, is_active, created_at)
        VALUES
            ($1, $2, '1100', 'Accounts Receivable', 'asset'::account_type, 'debit'::normal_balance, true, NOW()),
            ($3, $2, '4000', 'Revenue', 'revenue'::account_type, 'credit'::normal_balance, true, NOW()),
            ($4, $2, '5000', 'Cost of Goods Sold', 'expense'::account_type, 'debit'::normal_balance, true, NOW())
        ON CONFLICT (tenant_id, code) DO NOTHING
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .execute(pool)
    .await
    .expect("setup accounts");

    // Create period
    let period_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO accounting_periods (id, tenant_id, period_start, period_end, is_closed, created_at)
        VALUES ($1, $2, '2024-03-01', '2024-03-31', false, NOW())
        "#,
    )
    .bind(period_id)
    .bind(tenant_id)
    .execute(pool)
    .await
    .expect("setup period");

    // Create journal entries
    let entry_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO journal_entries (id, tenant_id, source_module, source_event_id, source_subject,
                                     posted_at, currency, description, reference_type, reference_id, created_at)
        VALUES ($1, $2, 'ar', $3, 'gl.posting.requested',
                '2024-03-15T00:00:00Z', 'USD', 'Invoice #1001', 'AR_INVOICE', 'inv-1001', NOW())
        "#,
    )
    .bind(entry_id)
    .bind(tenant_id)
    .bind(Uuid::new_v4())
    .execute(pool)
    .await
    .expect("setup journal entry");

    // Create journal lines
    sqlx::query(
        r#"
        INSERT INTO journal_lines (id, journal_entry_id, line_no, account_ref, debit_minor, credit_minor, memo)
        VALUES
            ($1, $2, 1, '1100', 250000, 0, 'AR debit'),
            ($3, $2, 2, '4000', 0, 250000, 'Revenue credit')
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(entry_id)
    .bind(Uuid::new_v4())
    .execute(pool)
    .await
    .expect("setup journal lines");

    period_id
}

async fn cleanup_export_data(pool: &PgPool, tenant_id: &str) {
    sqlx::query("DELETE FROM gl_exports WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM events_outbox WHERE aggregate_type = 'gl_export' AND aggregate_id IN (SELECT id::text FROM gl_exports WHERE tenant_id = $1)")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    // Clean outbox for this tenant's exports
    sqlx::query("DELETE FROM events_outbox WHERE event_type = 'gl.export.completed'")
        .execute(pool)
        .await
        .ok();
    sqlx::query(
        "DELETE FROM journal_lines WHERE journal_entry_id IN (SELECT id FROM journal_entries WHERE tenant_id = $1)",
    )
    .bind(tenant_id)
    .execute(pool)
    .await
    .ok();
    sqlx::query("DELETE FROM journal_entries WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM accounts WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM accounting_periods WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM account_balances WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
}

// ---------------------------------------------------------------------------
// 1. QuickBooks export E2E
// ---------------------------------------------------------------------------

#[tokio::test]
#[serial]
async fn test_quickbooks_coa_export() {
    let pool = get_test_pool().await;
    let tenant = "tenant-export-qb-coa";
    cleanup_export_data(&pool, tenant).await;
    setup_export_data(&pool, tenant).await;

    let result = execute_export(
        &pool,
        ExportRequest {
            tenant_id: tenant.to_string(),
            format: ExportFormat::QuickBooks,
            export_type: ExportType::ChartOfAccounts,
            idempotency_key: "qb-coa-001".to_string(),
            period_id: None,
        },
    )
    .await
    .expect("QB COA export should succeed");

    assert!(result.output.contains("!ACCNT\tNAME\tACCNTTYPE\tACCNUM"));
    assert!(result.output.contains("Accounts Receivable"));
    assert!(result.output.contains("Revenue"));
    assert!(result.output.contains("1100"));
    assert!(result.output.contains("4000"));

    cleanup_export_data(&pool, tenant).await;
    pool.close().await;
}

#[tokio::test]
#[serial]
async fn test_quickbooks_journal_export() {
    let pool = get_test_pool().await;
    let tenant = "tenant-export-qb-je";
    cleanup_export_data(&pool, tenant).await;
    let period_id = setup_export_data(&pool, tenant).await;

    let result = execute_export(
        &pool,
        ExportRequest {
            tenant_id: tenant.to_string(),
            format: ExportFormat::QuickBooks,
            export_type: ExportType::JournalEntries,
            idempotency_key: "qb-je-001".to_string(),
            period_id: Some(period_id),
        },
    )
    .await
    .expect("QB journal export should succeed");

    assert!(result.output.contains("!TRNS"));
    assert!(result.output.contains("GENERAL JOURNAL"));
    assert!(result.output.contains("Accounts Receivable"));
    assert!(result.output.contains("2500.00"));
    assert!(result.output.contains("ENDTRNS"));

    cleanup_export_data(&pool, tenant).await;
    pool.close().await;
}

// ---------------------------------------------------------------------------
// 2. Xero export E2E
// ---------------------------------------------------------------------------

#[tokio::test]
#[serial]
async fn test_xero_coa_export() {
    let pool = get_test_pool().await;
    let tenant = "tenant-export-xero-coa";
    cleanup_export_data(&pool, tenant).await;
    setup_export_data(&pool, tenant).await;

    let result = execute_export(
        &pool,
        ExportRequest {
            tenant_id: tenant.to_string(),
            format: ExportFormat::Xero,
            export_type: ExportType::ChartOfAccounts,
            idempotency_key: "xero-coa-001".to_string(),
            period_id: None,
        },
    )
    .await
    .expect("Xero COA export should succeed");

    assert!(result.output.contains("*Code,*Name,*Type"));
    assert!(result.output.contains("1100,Accounts Receivable,CURRENT"));
    assert!(result.output.contains("4000,Revenue,REVENUE"));

    cleanup_export_data(&pool, tenant).await;
    pool.close().await;
}

#[tokio::test]
#[serial]
async fn test_xero_journal_export() {
    let pool = get_test_pool().await;
    let tenant = "tenant-export-xero-je";
    cleanup_export_data(&pool, tenant).await;
    let period_id = setup_export_data(&pool, tenant).await;

    let result = execute_export(
        &pool,
        ExportRequest {
            tenant_id: tenant.to_string(),
            format: ExportFormat::Xero,
            export_type: ExportType::JournalEntries,
            idempotency_key: "xero-je-001".to_string(),
            period_id: Some(period_id),
        },
    )
    .await
    .expect("Xero journal export should succeed");

    assert!(result.output.contains("*Date,*Description,*AccountCode,*Debit,*Credit,Reference"));
    assert!(result.output.contains("1100"));
    assert!(result.output.contains("2500.00"));
    assert!(result.output.contains("inv-1001"));

    cleanup_export_data(&pool, tenant).await;
    pool.close().await;
}

// ---------------------------------------------------------------------------
// 3. Tenant isolation
// ---------------------------------------------------------------------------

#[tokio::test]
#[serial]
async fn test_tenant_isolation() {
    let pool = get_test_pool().await;
    let tenant_a = "tenant-export-iso-a";
    let tenant_b = "tenant-export-iso-b";
    cleanup_export_data(&pool, tenant_a).await;
    cleanup_export_data(&pool, tenant_b).await;

    let period_a = setup_export_data(&pool, tenant_a).await;

    // Export as tenant_B (which has no data) — should get empty results
    let result_b = execute_export(
        &pool,
        ExportRequest {
            tenant_id: tenant_b.to_string(),
            format: ExportFormat::QuickBooks,
            export_type: ExportType::ChartOfAccounts,
            idempotency_key: "iso-b-coa".to_string(),
            period_id: None,
        },
    )
    .await
    .expect("Export for tenant B should succeed");

    // Should only have the header line, no data rows
    let lines: Vec<&str> = result_b.output.lines().collect();
    assert_eq!(lines.len(), 1, "Tenant B export should have header only (no accounts)");

    // Also test journal export with tenant_B using tenant_A's period (no entries for B)
    let result_b_je = execute_export(
        &pool,
        ExportRequest {
            tenant_id: tenant_b.to_string(),
            format: ExportFormat::Xero,
            export_type: ExportType::JournalEntries,
            idempotency_key: "iso-b-je".to_string(),
            period_id: Some(period_a),
        },
    )
    .await
    .expect("Journal export for tenant B should succeed");

    let je_lines: Vec<&str> = result_b_je.output.lines().collect();
    assert_eq!(
        je_lines.len(),
        1,
        "Tenant B journal export should have header only (no entries)"
    );

    cleanup_export_data(&pool, tenant_a).await;
    cleanup_export_data(&pool, tenant_b).await;
    pool.close().await;
}

// ---------------------------------------------------------------------------
// 4. Idempotency
// ---------------------------------------------------------------------------

#[tokio::test]
#[serial]
async fn test_idempotency() {
    let pool = get_test_pool().await;
    let tenant = "tenant-export-idemp";
    cleanup_export_data(&pool, tenant).await;
    setup_export_data(&pool, tenant).await;

    let key = "idemp-key-001";

    // First export
    let result1 = execute_export(
        &pool,
        ExportRequest {
            tenant_id: tenant.to_string(),
            format: ExportFormat::QuickBooks,
            export_type: ExportType::ChartOfAccounts,
            idempotency_key: key.to_string(),
            period_id: None,
        },
    )
    .await
    .expect("First export should succeed");

    // Second export with same idempotency key — should return same result, no duplicate
    let result2 = execute_export(
        &pool,
        ExportRequest {
            tenant_id: tenant.to_string(),
            format: ExportFormat::QuickBooks,
            export_type: ExportType::ChartOfAccounts,
            idempotency_key: key.to_string(),
            period_id: None,
        },
    )
    .await
    .expect("Second export with same key should succeed (idempotent)");

    assert_eq!(result1.export_id, result2.export_id, "Same export ID returned");
    assert_eq!(result1.output, result2.output, "Same output returned");

    // Verify only one record in the database
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM gl_exports WHERE tenant_id = $1 AND idempotency_key = $2",
    )
    .bind(tenant)
    .bind(key)
    .fetch_one(&pool)
    .await
    .expect("count query");

    assert_eq!(count, 1, "Only one export record should exist (no duplicates)");

    cleanup_export_data(&pool, tenant).await;
    pool.close().await;
}

// ---------------------------------------------------------------------------
// 5. Outbox event verification
// ---------------------------------------------------------------------------

#[tokio::test]
#[serial]
async fn test_outbox_event() {
    let pool = get_test_pool().await;
    let tenant = "tenant-export-outbox";
    cleanup_export_data(&pool, tenant).await;
    setup_export_data(&pool, tenant).await;

    let result = execute_export(
        &pool,
        ExportRequest {
            tenant_id: tenant.to_string(),
            format: ExportFormat::Xero,
            export_type: ExportType::ChartOfAccounts,
            idempotency_key: "outbox-001".to_string(),
            period_id: None,
        },
    )
    .await
    .expect("Export should succeed");

    // Verify outbox contains the event
    let outbox_row: Option<(String, String, String)> = sqlx::query_as(
        r#"
        SELECT event_type, aggregate_type, aggregate_id
        FROM events_outbox
        WHERE aggregate_type = 'gl_export'
          AND aggregate_id = $1
        "#,
    )
    .bind(result.export_id.to_string())
    .fetch_optional(&pool)
    .await
    .expect("outbox query");

    let (event_type, agg_type, agg_id) = outbox_row.expect("Outbox event should exist");
    assert_eq!(event_type, EVENT_TYPE_EXPORT_COMPLETED);
    assert_eq!(agg_type, "gl_export");
    assert_eq!(agg_id, result.export_id.to_string());

    // Verify the payload contains tenant_id
    let payload: serde_json::Value = sqlx::query_scalar(
        "SELECT payload FROM events_outbox WHERE aggregate_type = 'gl_export' AND aggregate_id = $1",
    )
    .bind(result.export_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("payload query");

    let tenant_in_payload = payload
        .get("payload")
        .and_then(|p| p.get("tenant_id"))
        .and_then(|t| t.as_str());
    assert_eq!(tenant_in_payload, Some(tenant), "Outbox event should contain tenant_id");

    cleanup_export_data(&pool, tenant).await;
    pool.close().await;
}

// ---------------------------------------------------------------------------
// 6. Empty data export
// ---------------------------------------------------------------------------

#[tokio::test]
#[serial]
async fn test_empty_coa_export() {
    let pool = get_test_pool().await;
    let tenant = "tenant-export-empty";
    cleanup_export_data(&pool, tenant).await;

    // Export with no accounts — should succeed with header only
    let result = execute_export(
        &pool,
        ExportRequest {
            tenant_id: tenant.to_string(),
            format: ExportFormat::QuickBooks,
            export_type: ExportType::ChartOfAccounts,
            idempotency_key: "empty-coa-001".to_string(),
            period_id: None,
        },
    )
    .await
    .expect("Empty COA export should succeed");

    let lines: Vec<&str> = result.output.lines().collect();
    assert_eq!(lines.len(), 1, "Empty QB export should have only the header line");
    assert!(result.output.contains("!ACCNT"));

    cleanup_export_data(&pool, tenant).await;
    pool.close().await;
}

#[tokio::test]
#[serial]
async fn test_empty_journal_export() {
    let pool = get_test_pool().await;
    let tenant = "tenant-export-empty-je";
    cleanup_export_data(&pool, tenant).await;

    // Create a period but no journal entries
    let period_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO accounting_periods (id, tenant_id, period_start, period_end, is_closed, created_at)
        VALUES ($1, $2, '2024-04-01', '2024-04-30', false, NOW())
        "#,
    )
    .bind(period_id)
    .bind(tenant)
    .execute(&pool)
    .await
    .expect("setup empty period");

    let result = execute_export(
        &pool,
        ExportRequest {
            tenant_id: tenant.to_string(),
            format: ExportFormat::Xero,
            export_type: ExportType::JournalEntries,
            idempotency_key: "empty-je-001".to_string(),
            period_id: Some(period_id),
        },
    )
    .await
    .expect("Empty journal export should succeed");

    let lines: Vec<&str> = result.output.lines().collect();
    assert_eq!(lines.len(), 1, "Empty Xero journal export should have only the header");
    assert!(result.output.contains("*Date,*Description,*AccountCode"));

    cleanup_export_data(&pool, tenant).await;
    pool.close().await;
}
