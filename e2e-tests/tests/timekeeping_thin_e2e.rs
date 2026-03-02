//! E2E: Timekeeping module thin coverage (bd-5600)
//!
//! Validates the core timekeeping workflow end-to-end against a real
//! PostgreSQL database:
//!
//! 1. Create employee + project via direct service/SQL
//! 2. Create timesheet entries with fixed ISO dates (no timezone dependency)
//! 3. Submit approval → approve → verify status is "approved"
//! 4. Verify entries are retrievable and minutes match
//! 5. Verify entry correction (append-only versioning)
//! 6. Negative: duplicate entry for same employee/date/project/task → Overlap
//! 7. Negative: minutes > 1440 → Validation error
//! 8. Negative: approve an already-approved request → InvalidTransition
//!
//! No mocks, no stubs — real timekeeping PostgreSQL (port 5447).
//! All timestamps are fixed NaiveDate values. No use of Utc::now() in assertions.

mod common;

use anyhow::Result;
use chrono::NaiveDate;
use common::get_timekeeping_pool;
use sqlx::PgPool;
use timekeeping::domain::{
    approvals::{
        models::{ApprovalError, ApprovalStatus, ReviewApprovalRequest, SubmitApprovalRequest},
        service as approval_svc,
    },
    entries::{
        models::{CorrectEntryRequest, CreateEntryRequest, EntryError, EntryType},
        service as entry_svc,
    },
};
use uuid::Uuid;

// ============================================================================
// Constants — fixed dates, no timezone dependency
// ============================================================================

const FIXED_YEAR: i32 = 2026;
const FIXED_MONTH: u32 = 6;

fn fixed_date(day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(FIXED_YEAR, FIXED_MONTH, day).unwrap()
}

fn period_start() -> NaiveDate {
    fixed_date(1)
}

fn period_end() -> NaiveDate {
    fixed_date(7)
}

// ============================================================================
// Test helpers
// ============================================================================

const MIGRATION_LOCK_KEY: i64 = 7_447_560_001_i64;

async fn ensure_migrations(pool: &PgPool) {
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(MIGRATION_LOCK_KEY)
        .execute(pool)
        .await
        .expect("advisory lock failed");

    sqlx::migrate!("../modules/timekeeping/db/migrations")
        .run(pool)
        .await
        .expect("timekeeping migrations failed");

    sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(MIGRATION_LOCK_KEY)
        .execute(pool)
        .await
        .expect("advisory unlock failed");
}

/// Generate a unique app_id scoped to this test run.
fn unique_app_id(label: &str) -> String {
    let id = format!("tk-thin-{}-{}", label, Uuid::new_v4().simple());
    id[..50.min(id.len())].to_string()
}

/// Create an employee directly in the DB, returning its UUID.
async fn create_employee(pool: &PgPool, app_id: &str) -> Result<Uuid> {
    let code = format!(
        "EMP-{}",
        Uuid::new_v4().simple().to_string()[..8].to_uppercase()
    );
    let id: (Uuid,) = sqlx::query_as(
        r#"INSERT INTO tk_employees
            (app_id, employee_code, first_name, last_name,
             hourly_rate_minor, currency)
        VALUES ($1, $2, 'Test', 'Employee', 5000, 'USD')
        RETURNING id"#,
    )
    .bind(app_id)
    .bind(&code)
    .fetch_one(pool)
    .await?;
    Ok(id.0)
}

/// Create a project directly in the DB, returning its UUID.
async fn create_project(pool: &PgPool, app_id: &str) -> Result<Uuid> {
    let code = format!(
        "PROJ-{}",
        Uuid::new_v4().simple().to_string()[..8].to_uppercase()
    );
    let id: (Uuid,) = sqlx::query_as(
        r#"INSERT INTO tk_projects (app_id, project_code, name, billable)
        VALUES ($1, $2, 'Test Project', TRUE)
        RETURNING id"#,
    )
    .bind(app_id)
    .bind(&code)
    .fetch_one(pool)
    .await?;
    Ok(id.0)
}

/// Clean up all test data for a given app_id.
async fn cleanup(pool: &PgPool, app_id: &str) {
    let tables = [
        "DELETE FROM tk_approval_actions WHERE approval_id IN (SELECT id FROM tk_approval_requests WHERE app_id = $1)",
        "DELETE FROM tk_approval_requests WHERE app_id = $1",
        "DELETE FROM tk_idempotency_keys WHERE app_id = $1",
        "DELETE FROM tk_timesheet_entries WHERE app_id = $1",
        "DELETE FROM tk_projects WHERE app_id = $1",
        "DELETE FROM tk_employees WHERE app_id = $1",
    ];
    for sql in tables {
        sqlx::query(sql).bind(app_id).execute(pool).await.ok();
    }
}

// ============================================================================
// Test 1: Full happy-path flow — create entries, submit, approve, verify
// ============================================================================

#[tokio::test]
async fn test_timekeeping_entry_approval_flow() -> Result<()> {
    let pool = get_timekeeping_pool().await;
    ensure_migrations(&pool).await;

    let app_id = unique_app_id("flow");
    cleanup(&pool, &app_id).await;

    let employee_id = create_employee(&pool, &app_id).await?;
    let project_id = create_project(&pool, &app_id).await?;
    let actor_id = Uuid::new_v4(); // reviewer

    // --- Create 3 entries: Mon/Tue/Wed of fixed week, 480 min each ---
    let mut entry_ids = Vec::new();
    for day in 1..=3u32 {
        let entry = entry_svc::create_entry(
            &pool,
            &CreateEntryRequest {
                app_id: app_id.clone(),
                employee_id,
                project_id: Some(project_id),
                task_id: None,
                work_date: fixed_date(day),
                minutes: 480,
                description: Some(format!("Work day {}", day)),
                created_by: Some(actor_id),
            },
            None,
        )
        .await?;

        assert_eq!(entry.minutes, 480);
        assert_eq!(entry.entry_type, EntryType::Original);
        assert!(entry.is_current);
        assert_eq!(entry.work_date, fixed_date(day));
        entry_ids.push(entry.entry_id);
    }

    // --- Verify list_entries returns all 3 ---
    let entries =
        entry_svc::list_entries(&pool, &app_id, employee_id, period_start(), period_end()).await?;
    assert_eq!(entries.len(), 3, "Should have 3 current entries");
    let total_minutes: i32 = entries.iter().map(|e| e.minutes).sum();
    assert_eq!(total_minutes, 1440, "3 × 480 = 1440 minutes");

    // --- Submit approval ---
    let approval = approval_svc::submit(
        &pool,
        &SubmitApprovalRequest {
            app_id: app_id.clone(),
            employee_id,
            period_start: period_start(),
            period_end: period_end(),
            actor_id,
        },
    )
    .await?;
    assert_eq!(approval.status, ApprovalStatus::Submitted);
    assert_eq!(approval.total_minutes, 1440);

    // --- Approve ---
    let approved = approval_svc::approve(
        &pool,
        &ReviewApprovalRequest {
            app_id: app_id.clone(),
            approval_id: approval.id,
            actor_id,
            notes: Some("LGTM".into()),
        },
    )
    .await?;
    assert_eq!(approved.status, ApprovalStatus::Approved);
    assert!(approved.reviewed_at.is_some());
    assert_eq!(approved.reviewer_id, Some(actor_id));
    assert_eq!(approved.reviewer_notes.as_deref(), Some("LGTM"));

    // --- Verify approval actions audit trail ---
    let actions = approval_svc::approval_actions(&pool, approval.id).await?;
    assert_eq!(actions.len(), 2, "submit + approve = 2 actions");
    assert_eq!(actions[0].action, "submit");
    assert_eq!(actions[1].action, "approve");

    cleanup(&pool, &app_id).await;
    Ok(())
}

// ============================================================================
// Test 2: Entry correction (append-only versioning)
// ============================================================================

#[tokio::test]
async fn test_timekeeping_entry_correction() -> Result<()> {
    let pool = get_timekeeping_pool().await;
    ensure_migrations(&pool).await;

    let app_id = unique_app_id("corr");
    cleanup(&pool, &app_id).await;

    let employee_id = create_employee(&pool, &app_id).await?;
    let project_id = create_project(&pool, &app_id).await?;

    // Create original entry: 480 min on June 2
    let original = entry_svc::create_entry(
        &pool,
        &CreateEntryRequest {
            app_id: app_id.clone(),
            employee_id,
            project_id: Some(project_id),
            task_id: None,
            work_date: fixed_date(2),
            minutes: 480,
            description: Some("Original".into()),
            created_by: None,
        },
        None,
    )
    .await?;
    assert_eq!(original.version, 1);

    // Correct to 360 min
    let corrected = entry_svc::correct_entry(
        &pool,
        &CorrectEntryRequest {
            app_id: app_id.clone(),
            entry_id: original.entry_id,
            minutes: 360,
            description: Some("Corrected to 6h".into()),
            project_id: None,
            task_id: None,
            created_by: None,
        },
        None,
    )
    .await?;
    assert_eq!(corrected.version, 2);
    assert_eq!(corrected.minutes, 360);
    assert_eq!(corrected.entry_type, EntryType::Correction);
    assert!(corrected.is_current);

    // Verify history shows 2 versions
    let history = entry_svc::entry_history(&pool, &app_id, original.entry_id).await?;
    assert_eq!(history.len(), 2, "Should have 2 versions in history");
    assert!(!history[0].is_current, "v1 should not be current");
    assert!(history[1].is_current, "v2 should be current");

    // list_entries should only return the corrected version
    let entries =
        entry_svc::list_entries(&pool, &app_id, employee_id, fixed_date(1), fixed_date(7)).await?;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].minutes, 360);

    cleanup(&pool, &app_id).await;
    Ok(())
}

// ============================================================================
// Test 3: Negative — duplicate entry (overlap detection)
// ============================================================================

#[tokio::test]
async fn test_timekeeping_overlap_rejected() -> Result<()> {
    let pool = get_timekeeping_pool().await;
    ensure_migrations(&pool).await;

    let app_id = unique_app_id("ovlp");
    cleanup(&pool, &app_id).await;

    let employee_id = create_employee(&pool, &app_id).await?;
    let project_id = create_project(&pool, &app_id).await?;

    let req = CreateEntryRequest {
        app_id: app_id.clone(),
        employee_id,
        project_id: Some(project_id),
        task_id: None,
        work_date: fixed_date(3),
        minutes: 480,
        description: Some("First entry".into()),
        created_by: None,
    };

    // First entry succeeds
    entry_svc::create_entry(&pool, &req, None).await?;

    // Duplicate for same employee + date + project + task → Overlap
    let duplicate = entry_svc::create_entry(
        &pool,
        &CreateEntryRequest {
            app_id: app_id.clone(),
            employee_id,
            project_id: Some(project_id),
            task_id: None,
            work_date: fixed_date(3),
            minutes: 240,
            description: Some("Duplicate attempt".into()),
            created_by: None,
        },
        None,
    )
    .await;

    assert!(
        matches!(duplicate, Err(EntryError::Overlap)),
        "Expected Overlap error, got: {:?}",
        duplicate
    );

    cleanup(&pool, &app_id).await;
    Ok(())
}

// ============================================================================
// Test 4: Negative — invalid duration (exceeds 1440 minutes)
// ============================================================================

#[tokio::test]
async fn test_timekeeping_invalid_duration_rejected() -> Result<()> {
    let pool = get_timekeeping_pool().await;
    ensure_migrations(&pool).await;

    let app_id = unique_app_id("dur");
    cleanup(&pool, &app_id).await;

    let employee_id = create_employee(&pool, &app_id).await?;

    // Minutes > 1440 should fail validation
    let result = entry_svc::create_entry(
        &pool,
        &CreateEntryRequest {
            app_id: app_id.clone(),
            employee_id,
            project_id: None,
            task_id: None,
            work_date: fixed_date(4),
            minutes: 1441,
            description: None,
            created_by: None,
        },
        None,
    )
    .await;

    assert!(
        matches!(result, Err(EntryError::Validation(ref msg)) if msg.contains("1440")),
        "Expected validation error about max minutes, got: {:?}",
        result
    );

    // Negative minutes should also fail
    let neg_result = entry_svc::create_entry(
        &pool,
        &CreateEntryRequest {
            app_id: app_id.clone(),
            employee_id,
            project_id: None,
            task_id: None,
            work_date: fixed_date(5),
            minutes: -1,
            description: None,
            created_by: None,
        },
        None,
    )
    .await;

    assert!(
        matches!(neg_result, Err(EntryError::Validation(ref msg)) if msg.contains("negative")),
        "Expected validation error about negative minutes, got: {:?}",
        neg_result
    );

    cleanup(&pool, &app_id).await;
    Ok(())
}

// ============================================================================
// Test 5: Negative — approve an already-approved request
// ============================================================================

#[tokio::test]
async fn test_timekeeping_double_approve_rejected() -> Result<()> {
    let pool = get_timekeeping_pool().await;
    ensure_migrations(&pool).await;

    let app_id = unique_app_id("dbl");
    cleanup(&pool, &app_id).await;

    let employee_id = create_employee(&pool, &app_id).await?;
    let actor_id = Uuid::new_v4();

    // Create an entry so submission has non-zero minutes
    entry_svc::create_entry(
        &pool,
        &CreateEntryRequest {
            app_id: app_id.clone(),
            employee_id,
            project_id: None,
            task_id: None,
            work_date: fixed_date(1),
            minutes: 60,
            description: None,
            created_by: None,
        },
        None,
    )
    .await?;

    // Submit → approve
    let approval = approval_svc::submit(
        &pool,
        &SubmitApprovalRequest {
            app_id: app_id.clone(),
            employee_id,
            period_start: period_start(),
            period_end: period_end(),
            actor_id,
        },
    )
    .await?;

    approval_svc::approve(
        &pool,
        &ReviewApprovalRequest {
            app_id: app_id.clone(),
            approval_id: approval.id,
            actor_id,
            notes: None,
        },
    )
    .await?;

    // Second approve → InvalidTransition
    let result = approval_svc::approve(
        &pool,
        &ReviewApprovalRequest {
            app_id: app_id.clone(),
            approval_id: approval.id,
            actor_id,
            notes: None,
        },
    )
    .await;

    assert!(
        matches!(result, Err(ApprovalError::InvalidTransition { .. })),
        "Expected InvalidTransition, got: {:?}",
        result
    );

    cleanup(&pool, &app_id).await;
    Ok(())
}

// ============================================================================
// Test 6: Period lock — entries blocked after approval
// ============================================================================

#[tokio::test]
async fn test_timekeeping_period_lock_blocks_entries() -> Result<()> {
    let pool = get_timekeeping_pool().await;
    ensure_migrations(&pool).await;

    let app_id = unique_app_id("lock");
    cleanup(&pool, &app_id).await;

    let employee_id = create_employee(&pool, &app_id).await?;
    let project_id = create_project(&pool, &app_id).await?;
    let actor_id = Uuid::new_v4();

    // Create an entry, submit and approve the period
    entry_svc::create_entry(
        &pool,
        &CreateEntryRequest {
            app_id: app_id.clone(),
            employee_id,
            project_id: Some(project_id),
            task_id: None,
            work_date: fixed_date(2),
            minutes: 480,
            description: None,
            created_by: None,
        },
        None,
    )
    .await?;

    let approval = approval_svc::submit(
        &pool,
        &SubmitApprovalRequest {
            app_id: app_id.clone(),
            employee_id,
            period_start: period_start(),
            period_end: period_end(),
            actor_id,
        },
    )
    .await?;

    approval_svc::approve(
        &pool,
        &ReviewApprovalRequest {
            app_id: app_id.clone(),
            approval_id: approval.id,
            actor_id,
            notes: None,
        },
    )
    .await?;

    // Now try to create a new entry in the locked period
    let result = entry_svc::create_entry(
        &pool,
        &CreateEntryRequest {
            app_id: app_id.clone(),
            employee_id,
            project_id: None,
            task_id: None,
            work_date: fixed_date(3), // within approved period
            minutes: 60,
            description: None,
            created_by: None,
        },
        None,
    )
    .await;

    assert!(
        matches!(result, Err(EntryError::PeriodLocked(_))),
        "Expected PeriodLocked error, got: {:?}",
        result
    );

    cleanup(&pool, &app_id).await;
    Ok(())
}
