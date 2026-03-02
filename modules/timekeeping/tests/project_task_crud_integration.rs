//! Integration tests for project and task CRUD (bd-3l2v).
//!
//! Covers:
//! 1. Create project — happy path
//! 2. Duplicate project_code rejected
//! 3. Update project fields
//! 4. Deactivate project
//! 5. Create task under project — happy path
//! 6. Create task for wrong (cross-tenant) project rejected
//! 7. Deactivate task

use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use timekeeping::domain::projects::models::{
    CreateProjectRequest, CreateTaskRequest, ProjectError, TaskError, UpdateProjectRequest,
    UpdateTaskRequest,
};
use timekeeping::domain::projects::service::{ProjectRepo, TaskRepo};
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
    format!("proj-test-{}", Uuid::new_v4().simple())
}

fn base_project(app_id: &str, code: &str) -> CreateProjectRequest {
    CreateProjectRequest {
        app_id: app_id.to_string(),
        project_code: code.to_string(),
        name: format!("Project {code}"),
        description: Some("Test project".to_string()),
        billable: Some(true),
        gl_account_ref: Some("4000".to_string()),
    }
}

// ============================================================================
// 1. Create project — happy path
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_project() {
    let pool = setup_db().await;
    let app_id = unique_app();

    let proj = ProjectRepo::create(&pool, &base_project(&app_id, "PROJ-001"))
        .await
        .unwrap();

    assert_eq!(proj.app_id, app_id);
    assert_eq!(proj.project_code, "PROJ-001");
    assert_eq!(proj.name, "Project PROJ-001");
    assert!(proj.billable);
    assert!(proj.active);
}

// ============================================================================
// 2. Duplicate project_code rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_project_duplicate_code_rejected() {
    let pool = setup_db().await;
    let app_id = unique_app();

    ProjectRepo::create(&pool, &base_project(&app_id, "PROJ-DUP"))
        .await
        .unwrap();

    let err = ProjectRepo::create(&pool, &base_project(&app_id, "PROJ-DUP"))
        .await
        .unwrap_err();

    assert!(
        matches!(err, ProjectError::DuplicateCode(_, _)),
        "Expected DuplicateCode, got: {err:?}"
    );
}

// ============================================================================
// 3. Update project fields
// ============================================================================

#[tokio::test]
#[serial]
async fn test_update_project() {
    let pool = setup_db().await;
    let app_id = unique_app();

    let proj = ProjectRepo::create(&pool, &base_project(&app_id, "PROJ-UPD"))
        .await
        .unwrap();

    let updated = ProjectRepo::update(
        &pool,
        proj.id,
        &UpdateProjectRequest {
            app_id: app_id.clone(),
            name: Some("Updated Project Name".to_string()),
            description: Some("Updated desc".to_string()),
            billable: Some(false),
            gl_account_ref: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(updated.name, "Updated Project Name");
    assert_eq!(updated.description.as_deref(), Some("Updated desc"));
    assert!(!updated.billable);
    // gl_account_ref unchanged
    assert_eq!(updated.gl_account_ref.as_deref(), Some("4000"));
}

// ============================================================================
// 4. Deactivate project
// ============================================================================

#[tokio::test]
#[serial]
async fn test_deactivate_project() {
    let pool = setup_db().await;
    let app_id = unique_app();

    let proj = ProjectRepo::create(&pool, &base_project(&app_id, "PROJ-DEA"))
        .await
        .unwrap();
    assert!(proj.active);

    let deactivated = ProjectRepo::deactivate(&pool, proj.id, &app_id)
        .await
        .unwrap();
    assert!(!deactivated.active);

    let active_list = ProjectRepo::list(&pool, &app_id, true).await.unwrap();
    assert!(
        active_list.iter().all(|p| p.id != proj.id),
        "Deactivated project must not appear in active_only list"
    );
}

// ============================================================================
// 5. Create task under project — happy path
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_task() {
    let pool = setup_db().await;
    let app_id = unique_app();

    let proj = ProjectRepo::create(&pool, &base_project(&app_id, "PROJ-TASK"))
        .await
        .unwrap();

    let task = TaskRepo::create(
        &pool,
        &CreateTaskRequest {
            app_id: app_id.clone(),
            project_id: proj.id,
            task_code: "TASK-001".to_string(),
            name: "Design Mockups".to_string(),
        },
    )
    .await
    .unwrap();

    assert_eq!(task.project_id, proj.id);
    assert_eq!(task.task_code, "TASK-001");
    assert_eq!(task.name, "Design Mockups");
    assert!(task.active);
}

// ============================================================================
// 6. Create task for wrong (non-existent/cross-tenant) project rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_task_wrong_project_rejected() {
    let pool = setup_db().await;
    let app_a = unique_app();
    let app_b = unique_app();

    // Create project in app_a
    let proj_a = ProjectRepo::create(&pool, &base_project(&app_a, "PROJ-CROSS"))
        .await
        .unwrap();

    // Try to create a task for app_b referencing app_a's project
    let err = TaskRepo::create(
        &pool,
        &CreateTaskRequest {
            app_id: app_b.clone(),
            project_id: proj_a.id,
            task_code: "TASK-X".to_string(),
            name: "Cross-tenant task".to_string(),
        },
    )
    .await
    .unwrap_err();

    assert!(
        matches!(err, TaskError::ProjectNotFound),
        "Expected ProjectNotFound, got: {err:?}"
    );
}

// ============================================================================
// 7. Update and deactivate task
// ============================================================================

#[tokio::test]
#[serial]
async fn test_update_and_deactivate_task() {
    let pool = setup_db().await;
    let app_id = unique_app();

    let proj = ProjectRepo::create(&pool, &base_project(&app_id, "PROJ-TDEA"))
        .await
        .unwrap();

    let task = TaskRepo::create(
        &pool,
        &CreateTaskRequest {
            app_id: app_id.clone(),
            project_id: proj.id,
            task_code: "TASK-D".to_string(),
            name: "Original Name".to_string(),
        },
    )
    .await
    .unwrap();

    // Update name
    let updated = TaskRepo::update(
        &pool,
        task.id,
        &UpdateTaskRequest {
            app_id: app_id.clone(),
            name: Some("Renamed Task".to_string()),
        },
    )
    .await
    .unwrap();
    assert_eq!(updated.name, "Renamed Task");

    // Deactivate
    let deactivated = TaskRepo::deactivate(&pool, task.id, &app_id).await.unwrap();
    assert!(!deactivated.active);

    let active_tasks = TaskRepo::list_for_project(&pool, proj.id, &app_id, true)
        .await
        .unwrap();
    assert!(
        active_tasks.iter().all(|t| t.id != task.id),
        "Deactivated task must not appear in active_only list"
    );
}

// ============================================================================
// 8. Duplicate task_code within same project rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn test_create_task_duplicate_code_rejected() {
    let pool = setup_db().await;
    let app_id = unique_app();

    let proj = ProjectRepo::create(&pool, &base_project(&app_id, "PROJ-TDUP"))
        .await
        .unwrap();

    TaskRepo::create(
        &pool,
        &CreateTaskRequest {
            app_id: app_id.clone(),
            project_id: proj.id,
            task_code: "TASK-DUP".to_string(),
            name: "First".to_string(),
        },
    )
    .await
    .unwrap();

    let err = TaskRepo::create(
        &pool,
        &CreateTaskRequest {
            app_id: app_id.clone(),
            project_id: proj.id,
            task_code: "TASK-DUP".to_string(),
            name: "Second".to_string(),
        },
    )
    .await
    .unwrap_err();

    assert!(
        matches!(err, TaskError::DuplicateCode(_, _)),
        "Expected DuplicateCode, got: {err:?}"
    );
}
