//! Integration tests for allocation CRUD (bd-3l2v).
//!
//! Allocations = planned minutes-per-week per employee/project.
//! No Guard→Mutation→Outbox; simple CRUD.
//!
//! Covers:
//! 1. Create allocation — happy path
//! 2. Update allocation
//! 3. Deactivate allocation
//! 4. List allocations with employee and project filters
//! 5. Create allocation with negative minutes rejected (Validation)
//! 6. Create allocation with end-before-start rejected (Validation)

use chrono::NaiveDate;
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use timekeeping::domain::allocations::models::{
    AllocationError, CreateAllocationRequest, UpdateAllocationRequest,
};
use timekeeping::domain::allocations::service::{
    create_allocation, deactivate_allocation, get_allocation, list_allocations, update_allocation,
};
use timekeeping::domain::employees::models::CreateEmployeeRequest;
use timekeeping::domain::employees::service::EmployeeRepo;
use timekeeping::domain::projects::models::CreateProjectRequest;
use timekeeping::domain::projects::service::ProjectRepo;
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
    format!("alloc-test-{}", Uuid::new_v4().simple())
}

async fn create_test_employee(pool: &sqlx::PgPool, app_id: &str) -> Uuid {
    let emp = EmployeeRepo::create(
        pool,
        &CreateEmployeeRequest {
            app_id: app_id.to_string(),
            employee_code: format!("ALE-{}", Uuid::new_v4().simple()),
            first_name: "Alloc".to_string(),
            last_name: "Tester".to_string(),
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

async fn create_test_project(pool: &sqlx::PgPool, app_id: &str) -> Uuid {
    let proj = ProjectRepo::create(
        pool,
        &CreateProjectRequest {
            app_id: app_id.to_string(),
            project_code: format!("ALP-{}", Uuid::new_v4().simple()),
            name: "Alloc Test Project".to_string(),
            description: None,
            billable: Some(true),
            gl_account_ref: None,
        },
    )
    .await
    .unwrap();
    proj.id
}

fn effective_from() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()
}

fn effective_to() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 6, 30).unwrap()
}

// ============================================================================
// 1. Create allocation — happy path
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_allocation() {
    let pool = setup_db().await;
    let app_id = unique_app();
    let emp_id = create_test_employee(&pool, &app_id).await;
    let proj_id = create_test_project(&pool, &app_id).await;

    let alloc = create_allocation(
        &pool,
        &CreateAllocationRequest {
            app_id: app_id.clone(),
            employee_id: emp_id,
            project_id: proj_id,
            task_id: None,
            allocated_minutes_per_week: 2400,
            effective_from: effective_from(),
            effective_to: Some(effective_to()),
        },
    )
    .await
    .unwrap();

    assert_eq!(alloc.app_id, app_id);
    assert_eq!(alloc.employee_id, emp_id);
    assert_eq!(alloc.project_id, proj_id);
    assert_eq!(alloc.allocated_minutes_per_week, 2400);
    assert!(alloc.active);
    assert_eq!(alloc.effective_to, Some(effective_to()));
}

// ============================================================================
// 2. Update allocation
// ============================================================================

#[tokio::test]
#[serial]
async fn test_update_allocation() {
    let pool = setup_db().await;
    let app_id = unique_app();
    let emp_id = create_test_employee(&pool, &app_id).await;
    let proj_id = create_test_project(&pool, &app_id).await;

    let alloc = create_allocation(
        &pool,
        &CreateAllocationRequest {
            app_id: app_id.clone(),
            employee_id: emp_id,
            project_id: proj_id,
            task_id: None,
            allocated_minutes_per_week: 1200,
            effective_from: effective_from(),
            effective_to: None,
        },
    )
    .await
    .unwrap();

    let new_end = NaiveDate::from_ymd_opt(2026, 3, 31).unwrap();
    let updated = update_allocation(
        &pool,
        alloc.id,
        &UpdateAllocationRequest {
            app_id: app_id.clone(),
            allocated_minutes_per_week: Some(1800),
            effective_to: Some(new_end),
        },
    )
    .await
    .unwrap();

    assert_eq!(updated.allocated_minutes_per_week, 1800);
    assert_eq!(updated.effective_to, Some(new_end));
}

// ============================================================================
// 3. Deactivate allocation
// ============================================================================

#[tokio::test]
#[serial]
async fn test_deactivate_allocation() {
    let pool = setup_db().await;
    let app_id = unique_app();
    let emp_id = create_test_employee(&pool, &app_id).await;
    let proj_id = create_test_project(&pool, &app_id).await;

    let alloc = create_allocation(
        &pool,
        &CreateAllocationRequest {
            app_id: app_id.clone(),
            employee_id: emp_id,
            project_id: proj_id,
            task_id: None,
            allocated_minutes_per_week: 600,
            effective_from: effective_from(),
            effective_to: None,
        },
    )
    .await
    .unwrap();
    assert!(alloc.active);

    let deactivated = deactivate_allocation(&pool, alloc.id, &app_id)
        .await
        .unwrap();
    assert!(!deactivated.active);

    // Update on inactive allocation → NotFound
    let err = update_allocation(
        &pool,
        alloc.id,
        &UpdateAllocationRequest {
            app_id: app_id.clone(),
            allocated_minutes_per_week: Some(999),
            effective_to: None,
        },
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, AllocationError::NotFound),
        "Update on deactivated allocation must return NotFound, got: {err:?}"
    );
}

// ============================================================================
// 4. List allocations with filters
// ============================================================================

#[tokio::test]
#[serial]
async fn test_list_allocations_with_filters() {
    let pool = setup_db().await;
    let app_id = unique_app();
    let emp1 = create_test_employee(&pool, &app_id).await;
    let emp2 = create_test_employee(&pool, &app_id).await;
    let proj_id = create_test_project(&pool, &app_id).await;

    create_allocation(
        &pool,
        &CreateAllocationRequest {
            app_id: app_id.clone(),
            employee_id: emp1,
            project_id: proj_id,
            task_id: None,
            allocated_minutes_per_week: 2400,
            effective_from: effective_from(),
            effective_to: None,
        },
    )
    .await
    .unwrap();

    create_allocation(
        &pool,
        &CreateAllocationRequest {
            app_id: app_id.clone(),
            employee_id: emp2,
            project_id: proj_id,
            task_id: None,
            allocated_minutes_per_week: 1200,
            effective_from: effective_from(),
            effective_to: None,
        },
    )
    .await
    .unwrap();

    // All allocations for app
    let all = list_allocations(&pool, &app_id, None, None, false)
        .await
        .unwrap();
    assert!(all.len() >= 2);

    // Filter by employee
    let emp1_allocs = list_allocations(&pool, &app_id, Some(emp1), None, false)
        .await
        .unwrap();
    assert_eq!(emp1_allocs.len(), 1);
    assert_eq!(emp1_allocs[0].employee_id, emp1);

    // Filter by project
    let proj_allocs = list_allocations(&pool, &app_id, None, Some(proj_id), false)
        .await
        .unwrap();
    assert!(proj_allocs.len() >= 2);
    assert!(proj_allocs.iter().all(|a| a.project_id == proj_id));
}

// ============================================================================
// 5. Negative minutes rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_allocation_negative_minutes_rejected() {
    let pool = setup_db().await;
    let app_id = unique_app();
    let emp_id = create_test_employee(&pool, &app_id).await;
    let proj_id = create_test_project(&pool, &app_id).await;

    let err = create_allocation(
        &pool,
        &CreateAllocationRequest {
            app_id: app_id.clone(),
            employee_id: emp_id,
            project_id: proj_id,
            task_id: None,
            allocated_minutes_per_week: -100,
            effective_from: effective_from(),
            effective_to: None,
        },
    )
    .await
    .unwrap_err();

    assert!(
        matches!(err, AllocationError::Validation(_)),
        "Expected Validation error for negative minutes, got: {err:?}"
    );
}

// ============================================================================
// 6. End-before-start rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_allocation_end_before_start_rejected() {
    let pool = setup_db().await;
    let app_id = unique_app();
    let emp_id = create_test_employee(&pool, &app_id).await;
    let proj_id = create_test_project(&pool, &app_id).await;

    let bad_end = NaiveDate::from_ymd_opt(2025, 12, 31).unwrap(); // before effective_from 2026-01-01

    let err = create_allocation(
        &pool,
        &CreateAllocationRequest {
            app_id: app_id.clone(),
            employee_id: emp_id,
            project_id: proj_id,
            task_id: None,
            allocated_minutes_per_week: 2400,
            effective_from: effective_from(),
            effective_to: Some(bad_end),
        },
    )
    .await
    .unwrap_err();

    assert!(
        matches!(err, AllocationError::Validation(_)),
        "Expected Validation error for end < start, got: {err:?}"
    );
}

// ============================================================================
// 7. Get allocation — not found across tenant boundary
// ============================================================================

#[tokio::test]
#[serial]
async fn test_get_allocation_tenant_isolation() {
    let pool = setup_db().await;
    let app_a = unique_app();
    let app_b = unique_app();
    let emp_a = create_test_employee(&pool, &app_a).await;
    let proj_a = create_test_project(&pool, &app_a).await;

    let alloc_a = create_allocation(
        &pool,
        &CreateAllocationRequest {
            app_id: app_a.clone(),
            employee_id: emp_a,
            project_id: proj_a,
            task_id: None,
            allocated_minutes_per_week: 2400,
            effective_from: effective_from(),
            effective_to: None,
        },
    )
    .await
    .unwrap();

    // App B cannot fetch app A's allocation
    let err = get_allocation(&pool, alloc_a.id, &app_b)
        .await
        .unwrap_err();
    assert!(
        matches!(err, AllocationError::NotFound),
        "App B must not see app A's allocation"
    );
}
