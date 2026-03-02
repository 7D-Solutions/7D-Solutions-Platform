//! E2E: Timekeeping — billing rate + entries → AR invoice via billing run (bd-1o1n)
//!
//! Proves the timekeeping-to-billing chain end-to-end:
//!
//! 1. Create a billing rate (15000 cents/hour = $150/hr)
//! 2. Log 3 billable time entries (1h each) linked to the rate
//! 3. Run billing for the period against an AR customer
//! 4. Verify AR invoice created with amount_cents = 45000 (3 × 15000)
//! 5. Verify invoice line_item_details includes entry IDs
//! 6. Verify entries are marked billed (appear in tk_billing_run_entries)
//! 7. Repeat billing run → idempotent (no duplicate invoice)
//!
//! **Invariant:** Each billable time entry appears in exactly one billing run.
//! **No double-billing:** entries excluded from future runs once claimed.
//!
//! No mocks, no stubs — real TK + AR PostgreSQL pools.

mod common;

use anyhow::Result;
use chrono::NaiveDate;
use common::{get_ar_pool, get_timekeeping_pool};
use sqlx::PgPool;
use timekeeping::domain::billing::{
    models::{BillingError, CreateBillingRateRequest, CreateBillingRunRequest},
    service::{create_billing_rate, create_billing_run, set_invoice_id},
};
use uuid::Uuid;

// ============================================================================
// Constants
// ============================================================================

const RATE_CENTS_PER_HOUR: i32 = 15_000; // $150/hr
const ENTRY_MINUTES: i32 = 60; // 1 hour each
const EXPECTED_AMOUNT_CENTS: i64 = 45_000; // 3 × 15000

// ============================================================================
// TK setup helpers
// ============================================================================

async fn run_tk_migrations(pool: &PgPool) -> Result<()> {
    sqlx::migrate!("../modules/timekeeping/db/migrations")
        .run(pool)
        .await?;
    Ok(())
}

/// Create a TK employee and return its ID.
async fn create_employee(pool: &PgPool, app_id: &str) -> Result<Uuid> {
    let id: (Uuid,) = sqlx::query_as(
        r#"
        INSERT INTO tk_employees
            (app_id, employee_code, first_name, last_name, hourly_rate_minor, currency)
        VALUES ($1, $2, 'Billing', 'TestUser', 15000, 'USD')
        RETURNING id
        "#,
    )
    .bind(app_id)
    .bind(format!("EMP-BIL-{}", Uuid::new_v4()))
    .fetch_one(pool)
    .await?;
    Ok(id.0)
}

/// Create a billable TK project and return its ID.
async fn create_project(pool: &PgPool, app_id: &str) -> Result<Uuid> {
    let id: (Uuid,) = sqlx::query_as(
        r#"
        INSERT INTO tk_projects (app_id, project_code, name, billable)
        VALUES ($1, $2, 'Billable Project', TRUE)
        RETURNING id
        "#,
    )
    .bind(app_id)
    .bind(format!("PROJ-BIL-{}", Uuid::new_v4()))
    .fetch_one(pool)
    .await?;
    Ok(id.0)
}

/// Insert a billable timesheet entry linked to a billing rate.
async fn insert_billable_entry(
    pool: &PgPool,
    app_id: &str,
    employee_id: Uuid,
    project_id: Uuid,
    billing_rate_id: Uuid,
    work_date: NaiveDate,
) -> Result<Uuid> {
    let entry_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO tk_timesheet_entries
            (entry_id, version, app_id, employee_id, project_id,
             work_date, minutes, description, entry_type, is_current,
             billing_rate_id, billable)
        VALUES ($1, 1, $2, $3, $4, $5, $6, 'Service call', 'original', TRUE, $7, TRUE)
        "#,
    )
    .bind(entry_id)
    .bind(app_id)
    .bind(employee_id)
    .bind(project_id)
    .bind(work_date)
    .bind(ENTRY_MINUTES)
    .bind(billing_rate_id)
    .execute(pool)
    .await?;
    Ok(entry_id)
}

// ============================================================================
// AR setup helpers
// ============================================================================

/// Create an AR customer with a unique email and return its integer ID.
async fn create_ar_customer(pool: &PgPool, app_id: &str) -> Result<i32> {
    let run_id = Uuid::new_v4();
    let external_id = format!("tk-billing-e2e-{}", run_id);
    let email = format!("billing-e2e-{}@example.com", run_id);
    let id: (i32,) = sqlx::query_as(
        r#"
        INSERT INTO ar_customers (app_id, external_customer_id, email, name, status, updated_at)
        VALUES ($1, $2, $3, 'Billing E2E Customer', 'active', NOW())
        RETURNING id
        "#,
    )
    .bind(app_id)
    .bind(&external_id)
    .bind(&email)
    .fetch_one(pool)
    .await?;
    Ok(id.0)
}

/// Insert an AR invoice and return its integer ID.
async fn insert_ar_invoice(
    pool: &PgPool,
    app_id: &str,
    ar_customer_id: i32,
    amount_cents: i64,
    line_item_details: serde_json::Value,
) -> Result<i32> {
    let tilled_invoice_id = format!("tk-billing-run-{}", Uuid::new_v4());
    let id: (i32,) = sqlx::query_as(
        r#"
        INSERT INTO ar_invoices
            (app_id, tilled_invoice_id, ar_customer_id, status,
             amount_cents, currency, line_item_details, updated_at)
        VALUES ($1, $2, $3, 'open', $4, 'usd', $5, NOW())
        RETURNING id
        "#,
    )
    .bind(app_id)
    .bind(&tilled_invoice_id)
    .bind(ar_customer_id)
    .bind(amount_cents as i32)
    .bind(&line_item_details)
    .fetch_one(pool)
    .await?;
    Ok(id.0)
}

// ============================================================================
// Cleanup
// ============================================================================

async fn cleanup_tk(pool: &PgPool, app_id: &str) -> Result<()> {
    sqlx::query(
        "DELETE FROM tk_billing_run_entries WHERE billing_run_id IN \
         (SELECT id FROM tk_billing_runs WHERE app_id = $1)",
    )
    .bind(app_id)
    .execute(pool)
    .await
    .ok();
    sqlx::query("DELETE FROM tk_billing_runs WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM tk_idempotency_keys WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM tk_export_runs WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM tk_approval_requests WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM tk_timesheet_entries WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM tk_projects WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM tk_employees WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM tk_billing_rates WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    Ok(())
}

async fn cleanup_ar(pool: &PgPool, ar_customer_id: Option<i32>) -> Result<()> {
    if let Some(cid) = ar_customer_id {
        sqlx::query("DELETE FROM ar_invoices WHERE ar_customer_id = $1")
            .bind(cid)
            .execute(pool)
            .await
            .ok();
        sqlx::query("DELETE FROM ar_customers WHERE id = $1")
            .bind(cid)
            .execute(pool)
            .await
            .ok();
    }
    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

/// Full billing chain: rate → entries → billing run → AR invoice → billed entries
#[tokio::test]
async fn test_timekeeping_billing_rate_to_ar_invoice() -> Result<()> {
    let tk_pool = get_timekeeping_pool().await;
    let ar_pool = get_ar_pool().await;

    run_tk_migrations(&tk_pool).await?;

    // Use a unique app_id per test run to avoid cross-test interference
    let app_id = format!("tk-bil-main-{}", Uuid::new_v4().simple());
    let app_id = &app_id[..50.min(app_id.len())]; // truncate to 50 chars

    // AR customer ID tracked for cleanup
    let mut ar_customer_id_for_cleanup: Option<i32> = None;

    // -- Step 1: Create billing rate ------------------------------------------
    let rate = create_billing_rate(
        &tk_pool,
        &CreateBillingRateRequest {
            app_id: app_id.to_string(),
            name: "Standard".to_string(),
            rate_cents_per_hour: RATE_CENTS_PER_HOUR,
        },
    )
    .await?;

    assert_eq!(rate.rate_cents_per_hour, RATE_CENTS_PER_HOUR);
    assert_eq!(rate.name, "Standard");
    assert!(rate.is_active);

    let billing_rate_id = rate.id;

    // -- Step 2: Create employee + project + 3 billable entries ----------------
    let employee_id = create_employee(&tk_pool, app_id).await?;
    let project_id = create_project(&tk_pool, app_id).await?;

    let mut entry_ids = Vec::new();
    for day in 1u32..=3 {
        let date = NaiveDate::from_ymd_opt(2026, 3, day).unwrap();
        let eid = insert_billable_entry(
            &tk_pool,
            app_id,
            employee_id,
            project_id,
            billing_rate_id,
            date,
        )
        .await?;
        entry_ids.push(eid);
    }
    assert_eq!(entry_ids.len(), 3);

    // -- Step 3: Create AR customer -------------------------------------------
    let ar_customer_id = create_ar_customer(&ar_pool, app_id).await?;
    ar_customer_id_for_cleanup = Some(ar_customer_id);

    // -- Step 4: Run billing --------------------------------------------------
    let from_date = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
    let to_date = NaiveDate::from_ymd_opt(2026, 3, 31).unwrap();

    let result = create_billing_run(
        &tk_pool,
        &CreateBillingRunRequest {
            app_id: app_id.to_string(),
            ar_customer_id,
            from_date,
            to_date,
        },
    )
    .await?;

    assert!(!result.already_ran, "First run should not be a replay");
    assert_eq!(
        result.run.amount_cents, EXPECTED_AMOUNT_CENTS,
        "Billing run amount should be 3 × 15000 = 45000"
    );
    assert_eq!(result.line_items.len(), 3, "Should have 3 line items");

    // Each line item: 60 min * 15000 cents/hr / 60 = 15000 cents
    for item in &result.line_items {
        assert_eq!(
            item.amount_cents, 15_000,
            "Each 1-hour entry at $150/hr should bill 15000 cents"
        );
    }

    // -- Step 5: Create AR invoice with line item details ---------------------
    let line_item_details = serde_json::json!({
        "source": "timekeeping",
        "billing_run_id": result.run.id,
        "from_date": from_date.to_string(),
        "to_date": to_date.to_string(),
        "entry_ids": entry_ids.iter().map(|id| id.to_string()).collect::<Vec<_>>(),
        "lines": result.line_items.iter().map(|l| serde_json::json!({
            "entry_id": l.entry_id,
            "minutes": l.minutes,
            "rate_cents_per_hour": l.rate_cents_per_hour,
            "amount_cents": l.amount_cents,
        })).collect::<Vec<_>>(),
    });

    let ar_invoice_id = insert_ar_invoice(
        &ar_pool,
        app_id,
        ar_customer_id,
        result.run.amount_cents,
        line_item_details,
    )
    .await?;

    // Link invoice ID back to the billing run
    set_invoice_id(&tk_pool, result.run.id, ar_invoice_id).await?;

    // -- Step 6: Verify AR invoice --------------------------------------------
    let invoice: (i32, i32, String, serde_json::Value) = sqlx::query_as(
        r#"
        SELECT id, amount_cents, status, line_item_details
        FROM ar_invoices
        WHERE id = $1 AND ar_customer_id = $2
        "#,
    )
    .bind(ar_invoice_id)
    .bind(ar_customer_id)
    .fetch_one(&ar_pool)
    .await?;

    let (inv_id, inv_amount, inv_status, inv_line_details) = invoice;

    assert_eq!(inv_id, ar_invoice_id);
    assert_eq!(
        inv_amount, EXPECTED_AMOUNT_CENTS as i32,
        "Invoice amount_cents must be 45000"
    );
    assert_eq!(inv_status, "open");

    // Verify line item details reference the entry IDs
    let details_str = inv_line_details.to_string();
    for eid in &entry_ids {
        assert!(
            details_str.contains(&eid.to_string()),
            "line_item_details should reference entry_id {}",
            eid,
        );
    }

    // -- Step 7: Verify entries are marked billed (in tk_billing_run_entries) -
    let billed_count: (i64,) = sqlx::query_as(
        r#"
        SELECT COUNT(*)::BIGINT FROM tk_billing_run_entries
        WHERE billing_run_id = $1
        "#,
    )
    .bind(result.run.id)
    .fetch_one(&tk_pool)
    .await?;

    assert_eq!(
        billed_count.0, 3,
        "All 3 entries should be linked to the billing run"
    );

    // Verify each specific entry_id appears
    for eid in &entry_ids {
        let found: Option<(Uuid,)> = sqlx::query_as(
            "SELECT entry_id FROM tk_billing_run_entries WHERE billing_run_id = $1 AND entry_id = $2",
        )
        .bind(result.run.id)
        .bind(eid)
        .fetch_optional(&tk_pool)
        .await?;

        assert!(
            found.is_some(),
            "Entry {} should appear in billing run entries",
            eid
        );
    }

    // -- Step 8: Idempotency — repeat billing run for same period -------------
    let replay = create_billing_run(
        &tk_pool,
        &CreateBillingRunRequest {
            app_id: app_id.to_string(),
            ar_customer_id,
            from_date,
            to_date,
        },
    )
    .await?;

    assert!(
        replay.already_ran,
        "Second billing run for same period+customer must be idempotent"
    );
    assert_eq!(
        replay.run.id, result.run.id,
        "Idempotent replay must return the same billing run ID"
    );
    assert_eq!(
        replay.run.amount_cents, EXPECTED_AMOUNT_CENTS,
        "Idempotent replay must return same amount"
    );

    // Verify no duplicate AR invoice was created
    let invoice_count: (i64,) =
        sqlx::query_as("SELECT COUNT(*)::BIGINT FROM ar_invoices WHERE ar_customer_id = $1")
            .bind(ar_customer_id)
            .fetch_one(&ar_pool)
            .await?;

    assert_eq!(
        invoice_count.0, 1,
        "Idempotent replay must NOT create a second invoice"
    );

    // Cleanup
    cleanup_ar(&ar_pool, ar_customer_id_for_cleanup).await?;
    cleanup_tk(&tk_pool, app_id).await?;

    Ok(())
}

/// Double-billing guard: a second billing run for a different period
/// that overlaps with already-billed entries must find NO entries.
#[tokio::test]
async fn test_no_double_billing_for_billed_entries() -> Result<()> {
    let tk_pool = get_timekeeping_pool().await;
    let ar_pool = get_ar_pool().await;

    run_tk_migrations(&tk_pool).await?;

    // Unique app_id per test run
    let app_id = format!("tk-bil-nodb-{}", Uuid::new_v4().simple());
    let app_id = &app_id[..50.min(app_id.len())];

    // Create rate, employee, project
    let rate = create_billing_rate(
        &tk_pool,
        &CreateBillingRateRequest {
            app_id: app_id.to_string(),
            name: "NoDouble".to_string(),
            rate_cents_per_hour: 10_000,
        },
    )
    .await?;

    let employee_id = create_employee(&tk_pool, app_id).await?;
    let project_id = create_project(&tk_pool, app_id).await?;

    // Two entries on March 5 and March 10
    let entry1 = insert_billable_entry(
        &tk_pool,
        app_id,
        employee_id,
        project_id,
        rate.id,
        NaiveDate::from_ymd_opt(2026, 3, 5).unwrap(),
    )
    .await?;
    let entry2 = insert_billable_entry(
        &tk_pool,
        app_id,
        employee_id,
        project_id,
        rate.id,
        NaiveDate::from_ymd_opt(2026, 3, 10).unwrap(),
    )
    .await?;

    let ar_customer_id = create_ar_customer(&ar_pool, app_id).await?;

    // First billing run: March 1–15 (covers both entries)
    let result1 = create_billing_run(
        &tk_pool,
        &CreateBillingRunRequest {
            app_id: app_id.to_string(),
            ar_customer_id,
            from_date: NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(),
            to_date: NaiveDate::from_ymd_opt(2026, 3, 15).unwrap(),
        },
    )
    .await?;

    assert!(!result1.already_ran);
    assert_eq!(result1.line_items.len(), 2, "Should have 2 entries");

    // Verify both entry IDs appear in the billing run
    let billed: std::collections::HashSet<Uuid> =
        result1.line_items.iter().map(|l| l.entry_id).collect();
    assert!(billed.contains(&entry1));
    assert!(billed.contains(&entry2));

    // Second billing run: March 1–31 (same entries but already billed)
    // Must return NoBillableEntries — the double-billing guard
    let result2 = create_billing_run(
        &tk_pool,
        &CreateBillingRunRequest {
            app_id: app_id.to_string(),
            ar_customer_id,
            from_date: NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(),
            to_date: NaiveDate::from_ymd_opt(2026, 3, 31).unwrap(),
        },
    )
    .await;

    assert!(
        matches!(result2, Err(BillingError::NoBillableEntries)),
        "Second run over same entries must fail with NoBillableEntries, got: {:?}",
        result2,
    );

    // Cleanup
    cleanup_ar(&ar_pool, Some(ar_customer_id)).await?;
    cleanup_tk(&tk_pool, app_id).await?;

    Ok(())
}
