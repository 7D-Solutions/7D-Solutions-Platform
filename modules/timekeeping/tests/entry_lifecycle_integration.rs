//! Integration tests for timesheet entry lifecycle (bd-3l2v).
//!
//! Entry state machine (append-only):
//!   original → correction (new row, same entry_id, version+1)
//!            → void      (new row, minutes=0, entry_type='void')
//!
//! Covers:
//! 1. Create entry — happy path, outbox event written
//! 2. Correct entry — new version replaces current
//! 3. Void entry — current row replaced with minutes=0
//! 4. Correct non-existent entry rejected (NotFound)
//! 5. Void non-existent entry rejected (NotFound)
//! 6. Overlap rejected — duplicate employee+date+project+task
//! 7. Tenant isolation — entries invisible across app boundaries

use chrono::NaiveDate;
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use timekeeping::domain::employees::models::CreateEmployeeRequest;
use timekeeping::domain::employees::service::EmployeeRepo;
use timekeeping::domain::entries::models::{
    CorrectEntryRequest, CreateEntryRequest, EntryError, EntryType, VoidEntryRequest,
};
use timekeeping::domain::entries::service::{
    correct_entry, create_entry, list_entries, void_entry,
};
use uuid::Uuid;

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://timekeeping_user:timekeeping_pass@localhost:5447/timekeeping_db".to_string()
    });

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to timekeeping test DB");

    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run timekeeping migrations");

    pool
}

fn unique_app() -> String {
    format!("entry-test-{}", Uuid::new_v4().simple())
}

async fn create_test_employee(pool: &sqlx::PgPool, app_id: &str) -> Uuid {
    let emp = EmployeeRepo::create(
        pool,
        &CreateEmployeeRequest {
            app_id: app_id.to_string(),
            employee_code: format!("E-{}", Uuid::new_v4().simple()),
            first_name: "Test".to_string(),
            last_name: "Employee".to_string(),
            email: None,
            department: None,
            external_payroll_id: None,
            hourly_rate_minor: None,
            currency: None,
        },
    )
    .await
    .unwrap();
    emp.id
}

fn work_date() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 1, 15).unwrap()
}

// ============================================================================
// 1. Create entry — happy path + outbox event
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_entry_happy_path() {
    let pool = setup_db().await;
    let app_id = unique_app();
    let emp_id = create_test_employee(&pool, &app_id).await;

    let entry = create_entry(
        &pool,
        &CreateEntryRequest {
            app_id: app_id.clone(),
            employee_id: emp_id,
            project_id: None,
            task_id: None,
            work_date: work_date(),
            minutes: 480,
            description: Some("Morning shift".to_string()),
            created_by: None,
        },
        None,
    )
    .await
    .unwrap();

    assert_eq!(entry.app_id, app_id);
    assert_eq!(entry.employee_id, emp_id);
    assert_eq!(entry.minutes, 480);
    assert_eq!(entry.version, 1);
    assert!(entry.is_current);
    assert_eq!(entry.entry_type, EntryType::Original);

    // Verify outbox event written
    let event: Option<(String,)> = sqlx::query_as(
        "SELECT event_type FROM events_outbox WHERE aggregate_id = $1",
    )
    .bind(entry.entry_id.to_string())
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert_eq!(
        event.unwrap().0,
        "timesheet_entry.created",
        "Outbox event must be written on entry create"
    );
}

// ============================================================================
// 2. Correct entry — new version, old marked not current
// ============================================================================

#[tokio::test]
#[serial]
async fn test_correct_entry() {
    let pool = setup_db().await;
    let app_id = unique_app();
    let emp_id = create_test_employee(&pool, &app_id).await;

    let original = create_entry(
        &pool,
        &CreateEntryRequest {
            app_id: app_id.clone(),
            employee_id: emp_id,
            project_id: None,
            task_id: None,
            work_date: work_date(),
            minutes: 300,
            description: Some("Original".to_string()),
            created_by: None,
        },
        None,
    )
    .await
    .unwrap();

    let corrected = correct_entry(
        &pool,
        &CorrectEntryRequest {
            app_id: app_id.clone(),
            entry_id: original.entry_id,
            minutes: 420,
            description: Some("Corrected".to_string()),
            project_id: None,
            task_id: None,
            created_by: None,
        },
        None,
    )
    .await
    .unwrap();

    assert_eq!(corrected.entry_id, original.entry_id);
    assert_eq!(corrected.version, 2);
    assert_eq!(corrected.minutes, 420);
    assert_eq!(corrected.entry_type, EntryType::Correction);
    assert!(corrected.is_current);

    // Current listing reflects new value
    let entries = list_entries(&pool, &app_id, emp_id, work_date(), work_date())
        .await
        .unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].minutes, 420);
    assert_eq!(entries[0].version, 2);
}

// ============================================================================
// 3. Void entry — minutes=0, entry_type=void, old not current
// ============================================================================

#[tokio::test]
#[serial]
async fn test_void_entry() {
    let pool = setup_db().await;
    let app_id = unique_app();
    let emp_id = create_test_employee(&pool, &app_id).await;

    let original = create_entry(
        &pool,
        &CreateEntryRequest {
            app_id: app_id.clone(),
            employee_id: emp_id,
            project_id: None,
            task_id: None,
            work_date: work_date(),
            minutes: 240,
            description: None,
            created_by: None,
        },
        None,
    )
    .await
    .unwrap();

    let voided = void_entry(
        &pool,
        &VoidEntryRequest {
            app_id: app_id.clone(),
            entry_id: original.entry_id,
            created_by: None,
        },
        None,
    )
    .await
    .unwrap();

    assert_eq!(voided.entry_id, original.entry_id);
    assert_eq!(voided.minutes, 0);
    assert_eq!(voided.entry_type, EntryType::Void);
    assert!(voided.is_current);

    // list_entries returns the current (void) row — minutes=0, entry_type=Void
    let entries = list_entries(&pool, &app_id, emp_id, work_date(), work_date())
        .await
        .unwrap();
    assert_eq!(entries.len(), 1, "Expected 1 current entry (the void row)");
    assert_eq!(entries[0].minutes, 0, "Void row must have 0 minutes");
    assert_eq!(entries[0].entry_type, EntryType::Void);
}

// ============================================================================
// 4. Correct non-existent entry → NotFound
// ============================================================================

#[tokio::test]
#[serial]
async fn test_correct_nonexistent_entry_rejected() {
    let pool = setup_db().await;
    let app_id = unique_app();
    let ghost_id = Uuid::new_v4();

    let err = correct_entry(
        &pool,
        &CorrectEntryRequest {
            app_id: app_id.clone(),
            entry_id: ghost_id,
            minutes: 120,
            description: None,
            project_id: None,
            task_id: None,
            created_by: None,
        },
        None,
    )
    .await
    .unwrap_err();

    assert!(
        matches!(err, EntryError::NotFound),
        "Expected NotFound, got: {err:?}"
    );
}

// ============================================================================
// 5. Void non-existent entry → NotFound
// ============================================================================

#[tokio::test]
#[serial]
async fn test_void_nonexistent_entry_rejected() {
    let pool = setup_db().await;
    let app_id = unique_app();
    let ghost_id = Uuid::new_v4();

    let err = void_entry(
        &pool,
        &VoidEntryRequest {
            app_id: app_id.clone(),
            entry_id: ghost_id,
            created_by: None,
        },
        None,
    )
    .await
    .unwrap_err();

    assert!(
        matches!(err, EntryError::NotFound),
        "Expected NotFound, got: {err:?}"
    );
}

// ============================================================================
// 6. Overlap rejected — same employee+date+project+task
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_entry_overlap_rejected() {
    let pool = setup_db().await;
    let app_id = unique_app();
    let emp_id = create_test_employee(&pool, &app_id).await;

    // First entry succeeds (no project — project_id=NULL is valid)
    create_entry(
        &pool,
        &CreateEntryRequest {
            app_id: app_id.clone(),
            employee_id: emp_id,
            project_id: None,
            task_id: None,
            work_date: work_date(),
            minutes: 120,
            description: None,
            created_by: None,
        },
        None,
    )
    .await
    .unwrap();

    // Second entry same employee+date+project(NULL) → Overlap
    let err = create_entry(
        &pool,
        &CreateEntryRequest {
            app_id: app_id.clone(),
            employee_id: emp_id,
            project_id: None,
            task_id: None,
            work_date: work_date(),
            minutes: 60,
            description: None,
            created_by: None,
        },
        None,
    )
    .await
    .unwrap_err();

    assert!(
        matches!(err, EntryError::Overlap),
        "Expected Overlap, got: {err:?}"
    );
}

// ============================================================================
// 7. Tenant isolation
// ============================================================================

#[tokio::test]
#[serial]
async fn test_entry_tenant_isolation() {
    let pool = setup_db().await;
    let app_a = unique_app();
    let app_b = unique_app();

    let emp_a = create_test_employee(&pool, &app_a).await;

    // Create entry under app_a
    let entry_a = create_entry(
        &pool,
        &CreateEntryRequest {
            app_id: app_a.clone(),
            employee_id: emp_a,
            project_id: None,
            task_id: None,
            work_date: work_date(),
            minutes: 480,
            description: None,
            created_by: None,
        },
        None,
    )
    .await
    .unwrap();

    // App B cannot correct app A's entry (NotFound because app_id scoping)
    let err = correct_entry(
        &pool,
        &CorrectEntryRequest {
            app_id: app_b.clone(),
            entry_id: entry_a.entry_id,
            minutes: 999,
            description: None,
            project_id: None,
            task_id: None,
            created_by: None,
        },
        None,
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, EntryError::NotFound),
        "App B must not be able to correct app A's entry"
    );

    // App B cannot void app A's entry
    let err = void_entry(
        &pool,
        &VoidEntryRequest {
            app_id: app_b.clone(),
            entry_id: entry_a.entry_id,
            created_by: None,
        },
        None,
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, EntryError::NotFound),
        "App B must not be able to void app A's entry"
    );
}
