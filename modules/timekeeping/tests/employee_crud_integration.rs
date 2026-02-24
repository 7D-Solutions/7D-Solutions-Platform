//! Integration tests for employee CRUD (bd-3l2v).
//!
//! Covers:
//! 1. Create employee — happy path
//! 2. Duplicate employee_code rejected
//! 3. Update employee fields
//! 4. Deactivate employee (idempotent)
//! 5. List employees with active_only filter
//! 6. Tenant isolation — employee is invisible across app boundaries

use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use timekeeping::domain::employees::models::{
    CreateEmployeeRequest, EmployeeError, UpdateEmployeeRequest,
};
use timekeeping::domain::employees::service::EmployeeRepo;
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
    format!("emp-test-{}", Uuid::new_v4().simple())
}

fn base_create(app_id: &str, code: &str) -> CreateEmployeeRequest {
    CreateEmployeeRequest {
        app_id: app_id.to_string(),
        employee_code: code.to_string(),
        first_name: "Jane".to_string(),
        last_name: "Doe".to_string(),
        email: Some("jane@example.com".to_string()),
        department: Some("Engineering".to_string()),
        external_payroll_id: None,
        hourly_rate_minor: Some(5000),
        currency: Some("USD".to_string()),
    }
}

// ============================================================================
// 1. Create employee — happy path
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_employee() {
    let pool = setup_db().await;
    let app_id = unique_app();

    let emp = EmployeeRepo::create(&pool, &base_create(&app_id, "EMP-001"))
        .await
        .unwrap();

    assert_eq!(emp.app_id, app_id);
    assert_eq!(emp.employee_code, "EMP-001");
    assert_eq!(emp.first_name, "Jane");
    assert_eq!(emp.last_name, "Doe");
    assert_eq!(emp.currency, "USD");
    assert!(emp.active);
}

// ============================================================================
// 2. Duplicate employee_code rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_employee_duplicate_code_rejected() {
    let pool = setup_db().await;
    let app_id = unique_app();

    EmployeeRepo::create(&pool, &base_create(&app_id, "EMP-DUP"))
        .await
        .unwrap();

    let err = EmployeeRepo::create(&pool, &base_create(&app_id, "EMP-DUP"))
        .await
        .unwrap_err();

    assert!(
        matches!(err, EmployeeError::DuplicateCode(_, _)),
        "Expected DuplicateCode, got: {err:?}"
    );
}

// ============================================================================
// 3. Update employee fields
// ============================================================================

#[tokio::test]
#[serial]
async fn test_update_employee() {
    let pool = setup_db().await;
    let app_id = unique_app();

    let emp = EmployeeRepo::create(&pool, &base_create(&app_id, "EMP-UPD"))
        .await
        .unwrap();

    let updated = EmployeeRepo::update(
        &pool,
        emp.id,
        &UpdateEmployeeRequest {
            app_id: app_id.clone(),
            first_name: Some("Updated".to_string()),
            last_name: Some("Name".to_string()),
            email: None,
            department: Some("HR".to_string()),
            external_payroll_id: Some("ADP-999".to_string()),
            hourly_rate_minor: Some(7500),
            currency: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(updated.first_name, "Updated");
    assert_eq!(updated.last_name, "Name");
    assert_eq!(updated.department.as_deref(), Some("HR"));
    assert_eq!(updated.hourly_rate_minor, Some(7500));
    // Currency unchanged
    assert_eq!(updated.currency, "USD");
}

// ============================================================================
// 4. Deactivate employee (idempotent)
// ============================================================================

#[tokio::test]
#[serial]
async fn test_deactivate_employee_idempotent() {
    let pool = setup_db().await;
    let app_id = unique_app();

    let emp = EmployeeRepo::create(&pool, &base_create(&app_id, "EMP-DEA"))
        .await
        .unwrap();

    let deactivated = EmployeeRepo::deactivate(&pool, emp.id, &app_id)
        .await
        .unwrap();
    assert!(!deactivated.active);

    // Second call is idempotent
    let deactivated2 = EmployeeRepo::deactivate(&pool, emp.id, &app_id)
        .await
        .unwrap();
    assert!(!deactivated2.active);
}

// ============================================================================
// 5. List employees with active_only filter
// ============================================================================

#[tokio::test]
#[serial]
async fn test_list_employees_active_only() {
    let pool = setup_db().await;
    let app_id = unique_app();

    let emp1 = EmployeeRepo::create(&pool, &base_create(&app_id, "EMP-L1"))
        .await
        .unwrap();
    let _emp2 = EmployeeRepo::create(&pool, &base_create(&app_id, "EMP-L2"))
        .await
        .unwrap();

    // Deactivate emp1
    EmployeeRepo::deactivate(&pool, emp1.id, &app_id)
        .await
        .unwrap();

    let all = EmployeeRepo::list(&pool, &app_id, false).await.unwrap();
    assert!(all.len() >= 2, "Expected at least 2 total employees");

    let active = EmployeeRepo::list(&pool, &app_id, true).await.unwrap();
    assert_eq!(active.len(), 1, "Expected exactly 1 active employee");
    assert_eq!(active[0].employee_code, "EMP-L2");
}

// ============================================================================
// 6. Tenant isolation
// ============================================================================

#[tokio::test]
#[serial]
async fn test_employee_tenant_isolation() {
    let pool = setup_db().await;
    let app_a = unique_app();
    let app_b = unique_app();

    let emp_a = EmployeeRepo::create(&pool, &base_create(&app_a, "EMP-ISO"))
        .await
        .unwrap();

    // App B cannot find app A's employee
    let result = EmployeeRepo::find_by_id(&pool, emp_a.id, &app_b)
        .await
        .unwrap();
    assert!(result.is_none(), "App B must not see app A's employee");

    // App B list returns empty
    let list_b = EmployeeRepo::list(&pool, &app_b, false).await.unwrap();
    assert!(
        list_b.iter().all(|e| e.id != emp_a.id),
        "App B list must not contain app A's employee"
    );

    // App B cannot update app A's employee
    let err = EmployeeRepo::update(
        &pool,
        emp_a.id,
        &UpdateEmployeeRequest {
            app_id: app_b.clone(),
            first_name: Some("Hacked".to_string()),
            last_name: None,
            email: None,
            department: None,
            external_payroll_id: None,
            hourly_rate_minor: None,
            currency: None,
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, EmployeeError::NotFound));
}
