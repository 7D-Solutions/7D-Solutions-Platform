//! GL Export behavior tests: idempotency + outbox event verification
//!
//! Test categories covered:
//! 4. Idempotency
//! 5. Outbox event verification

mod common;
mod gl_export_helpers;

use common::get_test_pool;
use gl_export_helpers::{cleanup_export_data, setup_export_data};
use gl_rs::exports::service::{
    execute_export, ExportFormat, ExportRequest, ExportType, EVENT_TYPE_EXPORT_COMPLETED,
};
use serial_test::serial;

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

    assert_eq!(
        result1.export_id, result2.export_id,
        "Same export ID returned"
    );
    assert_eq!(result1.output, result2.output, "Same output returned");

    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM gl_exports WHERE tenant_id = $1 AND idempotency_key = $2",
    )
    .bind(tenant)
    .bind(key)
    .fetch_one(&pool)
    .await
    .expect("count query");

    assert_eq!(count, 1, "Only one export record should exist");

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

    let outbox_row: Option<(String, String, String)> = sqlx::query_as(
        "SELECT event_type, aggregate_type, aggregate_id
         FROM events_outbox
         WHERE aggregate_type = 'gl_export' AND aggregate_id = $1",
    )
    .bind(result.export_id.to_string())
    .fetch_optional(&pool)
    .await
    .expect("outbox query");

    let (event_type, agg_type, agg_id) = outbox_row.expect("Outbox event should exist");
    assert_eq!(event_type, EVENT_TYPE_EXPORT_COMPLETED);
    assert_eq!(agg_type, "gl_export");
    assert_eq!(agg_id, result.export_id.to_string());

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
    assert_eq!(
        tenant_in_payload,
        Some(tenant),
        "Event should contain tenant_id"
    );

    cleanup_export_data(&pool, tenant).await;
    pool.close().await;
}
