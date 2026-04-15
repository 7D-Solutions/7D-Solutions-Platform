//! GL Export E2E tests: QuickBooks + Xero format verification
//!
//! Test categories covered:
//! 1. QuickBooks COA + journal entry export
//! 2. Xero COA + journal entry export
//! 3. Tenant isolation
//! 4. Empty data export

mod common;
mod gl_export_helpers;

use common::get_test_pool;
use gl_export_helpers::{cleanup_export_data, setup_export_data};
use gl_rs::exports::service::{execute_export, ExportFormat, ExportRequest, ExportType};
use serial_test::serial;
use uuid::Uuid;

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

    assert!(result
        .output
        .contains("*Date,*Description,*AccountCode,*Debit,*Credit,Reference"));
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

    let lines: Vec<&str> = result_b.output.lines().collect();
    assert_eq!(lines.len(), 1, "Tenant B export should have header only");

    // Journal export with tenant_B using tenant_A's period (no entries for B)
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
        "Tenant B journal export should have header only"
    );

    cleanup_export_data(&pool, tenant_a).await;
    cleanup_export_data(&pool, tenant_b).await;
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
    assert_eq!(
        lines.len(),
        1,
        "Empty QB export should have only the header"
    );
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

    let period_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO accounting_periods (id, tenant_id, period_start, period_end, is_closed, created_at)
         VALUES ($1, $2, '2024-04-01', '2024-04-30', false, NOW())",
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
    assert_eq!(
        lines.len(),
        1,
        "Empty Xero journal export should have only the header"
    );
    assert!(result.output.contains("*Date,*Description,*AccountCode"));

    cleanup_export_data(&pool, tenant).await;
    pool.close().await;
}
