//! Integration tests for reporting data export (CSV/Excel/PDF).
//!
//! All tests run against real Postgres (REPORTING_DATABASE_URL, port 5443).
//! No mocks, no stubs.

mod helpers;

use helpers::{seed_trial_balance, setup_db, unique_tenant};
use reporting::domain::exports::{
    models::ExportFormat,
    service::{list_export_runs, run_export},
};
use serial_test::serial;

// ═══════════════════════════════════════════════════════════════════════════════
// 1. CSV EXPORT E2E
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
#[serial]
async fn csv_export_produces_valid_output_with_correct_row_count() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let tid_str = tid.to_string();

    // Seed 3 trial-balance rows
    seed_trial_balance(
        &pool,
        &tid_str,
        "2026-01-31",
        "1000",
        "Cash",
        "USD",
        200_000,
        0,
    )
    .await;
    seed_trial_balance(
        &pool,
        &tid_str,
        "2026-01-31",
        "4000",
        "Revenue",
        "USD",
        0,
        100_000,
    )
    .await;
    seed_trial_balance(
        &pool,
        &tid_str,
        "2026-01-31",
        "6000",
        "Rent Expense",
        "USD",
        40_000,
        0,
    )
    .await;

    let run = run_export(&pool, &tid_str, "trial_balance", ExportFormat::Csv, None)
        .await
        .expect("CSV export should succeed");

    assert_eq!(run.status, "completed");
    assert_eq!(run.row_count, Some(3));
    assert!(run.output_ref.is_some());
    assert!(
        run.output_ref.as_ref().unwrap().ends_with(".csv"),
        "output_ref should end with .csv"
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// 2. EXCEL EXPORT E2E
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
#[serial]
async fn xlsx_export_produces_valid_output_with_correct_row_count() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let tid_str = tid.to_string();

    seed_trial_balance(
        &pool,
        &tid_str,
        "2026-01-31",
        "1000",
        "Cash",
        "USD",
        150_000,
        0,
    )
    .await;
    seed_trial_balance(
        &pool,
        &tid_str,
        "2026-01-31",
        "2000",
        "AP",
        "USD",
        0,
        50_000,
    )
    .await;

    let run = run_export(&pool, &tid_str, "trial_balance", ExportFormat::Xlsx, None)
        .await
        .expect("XLSX export should succeed");

    assert_eq!(run.status, "completed");
    assert_eq!(run.row_count, Some(2));
    assert!(run.output_ref.is_some());
    assert!(
        run.output_ref.as_ref().unwrap().ends_with(".xlsx"),
        "output_ref should end with .xlsx"
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// 3. PDF EXPORT E2E
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
#[serial]
async fn pdf_export_produces_valid_output_without_error() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let tid_str = tid.to_string();

    seed_trial_balance(
        &pool,
        &tid_str,
        "2026-01-31",
        "1000",
        "Cash",
        "USD",
        300_000,
        0,
    )
    .await;

    let run = run_export(&pool, &tid_str, "trial_balance", ExportFormat::Pdf, None)
        .await
        .expect("PDF export should succeed");

    assert_eq!(run.status, "completed");
    assert_eq!(run.row_count, Some(1));
    assert!(run.output_ref.is_some());
    assert!(
        run.output_ref.as_ref().unwrap().ends_with(".pdf"),
        "output_ref should end with .pdf"
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// 4. TENANT ISOLATION TEST
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
#[serial]
async fn tenant_isolation_export_runs_invisible_across_tenants() {
    let pool = setup_db().await;
    let tenant_a = unique_tenant().to_string();
    let tenant_b = unique_tenant().to_string();

    // Seed data and run export for tenant A
    seed_trial_balance(
        &pool,
        &tenant_a,
        "2026-01-31",
        "1000",
        "Cash",
        "USD",
        100_000,
        0,
    )
    .await;
    let run_a = run_export(&pool, &tenant_a, "trial_balance", ExportFormat::Csv, None)
        .await
        .expect("Tenant A export should succeed");
    assert_eq!(run_a.status, "completed");

    // Tenant B should see zero export runs
    let runs_b = list_export_runs(&pool, &tenant_b)
        .await
        .expect("list should succeed");
    assert!(
        runs_b.is_empty(),
        "Tenant B must not see tenant A's export runs"
    );

    // Tenant A should see their export run
    let runs_a = list_export_runs(&pool, &tenant_a)
        .await
        .expect("list should succeed");
    assert_eq!(runs_a.len(), 1);
    assert_eq!(runs_a[0].id, run_a.id);
}

// ═══════════════════════════════════════════════════════════════════════════════
// 5. IDEMPOTENCY TEST
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
#[serial]
async fn idempotent_export_request_does_not_create_duplicate() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let tid_str = tid.to_string();
    let key = format!("idem-{}", uuid::Uuid::new_v4());

    seed_trial_balance(
        &pool,
        &tid_str,
        "2026-01-31",
        "1000",
        "Cash",
        "USD",
        50_000,
        0,
    )
    .await;

    // First request
    let run1 = run_export(
        &pool,
        &tid_str,
        "trial_balance",
        ExportFormat::Csv,
        Some(&key),
    )
    .await
    .expect("First export should succeed");

    // Second request with same key
    let run2 = run_export(
        &pool,
        &tid_str,
        "trial_balance",
        ExportFormat::Csv,
        Some(&key),
    )
    .await
    .expect("Second export should return existing");

    assert_eq!(
        run1.id, run2.id,
        "Same idempotency key must return same run"
    );

    // Verify only one run exists
    let runs = list_export_runs(&pool, &tid_str)
        .await
        .expect("list should succeed");
    assert_eq!(runs.len(), 1, "No duplicate export run should be created");
}

// ═══════════════════════════════════════════════════════════════════════════════
// 6. OUTBOX EVENT TEST
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
#[serial]
async fn outbox_event_emitted_with_format_row_count_and_tenant_id() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let tid_str = tid.to_string();

    seed_trial_balance(
        &pool,
        &tid_str,
        "2026-01-31",
        "1000",
        "Cash",
        "USD",
        75_000,
        0,
    )
    .await;
    seed_trial_balance(
        &pool,
        &tid_str,
        "2026-01-31",
        "4000",
        "Revenue",
        "USD",
        0,
        60_000,
    )
    .await;

    let run = run_export(&pool, &tid_str, "trial_balance", ExportFormat::Xlsx, None)
        .await
        .expect("Export should succeed");

    // Query the outbox for the event
    let event: (String, serde_json::Value, String) = sqlx::query_as(
        r#"SELECT event_type, payload, tenant_id
           FROM events_outbox
           WHERE aggregate_type = 'export_run' AND aggregate_id = $1
           ORDER BY created_at DESC LIMIT 1"#,
    )
    .bind(run.id.to_string())
    .fetch_one(&pool)
    .await
    .expect("Outbox event should exist");

    let (event_type, payload, tenant_id) = event;

    assert_eq!(event_type, "reporting.export.completed");
    assert_eq!(tenant_id, tid_str);

    // Verify payload contains format, row_count, and tenant_id
    let inner = &payload["payload"];
    assert_eq!(inner["format"], "xlsx");
    assert_eq!(inner["row_count"], 2);
    assert_eq!(inner["export_run_id"], run.id.to_string());

    // Verify envelope has tenant_id
    assert_eq!(payload["tenant_id"], tid_str);
}
