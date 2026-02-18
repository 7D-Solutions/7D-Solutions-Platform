//! Project and Task HTTP handlers — CRUD endpoints for project/task catalog.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    domain::projects::{
        models::{
            CreateProjectRequest, CreateTaskRequest, ProjectError, TaskError,
            UpdateProjectRequest, UpdateTaskRequest,
        },
        service::{ProjectRepo, TaskRepo},
    },
    AppState,
};

// ============================================================================
// Query params
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct AppIdQuery {
    pub app_id: String,
    #[serde(default = "default_true")]
    pub active_only: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize)]
pub struct TaskListQuery {
    pub app_id: String,
    #[serde(default = "default_true")]
    pub active_only: bool,
}

// ============================================================================
// Project error mapping
// ============================================================================

fn project_error_response(err: ProjectError) -> impl IntoResponse {
    match err {
        ProjectError::NotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "Project not found" })),
        ),
        ProjectError::DuplicateCode(code, app) => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "duplicate_code",
                "message": format!("Project code '{}' already exists for app '{}'", code, app)
            })),
        ),
        ProjectError::Validation(msg) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "validation_error", "message": msg })),
        ),
        ProjectError::Database(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "database_error", "message": e.to_string() })),
        ),
    }
}

// ============================================================================
// Task error mapping
// ============================================================================

fn task_error_response(err: TaskError) -> impl IntoResponse {
    match err {
        TaskError::NotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "Task not found" })),
        ),
        TaskError::ProjectNotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "project_not_found", "message": "Parent project not found" })),
        ),
        TaskError::DuplicateCode(code, project_id) => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "duplicate_code",
                "message": format!("Task code '{}' already exists for project '{}'", code, project_id)
            })),
        ),
        TaskError::Validation(msg) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "validation_error", "message": msg })),
        ),
        TaskError::Database(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "database_error", "message": e.to_string() })),
        ),
    }
}

// ============================================================================
// Project handlers
// ============================================================================

/// POST /api/timekeeping/projects
pub async fn create_project(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateProjectRequest>,
) -> impl IntoResponse {
    match ProjectRepo::create(&state.pool, &req).await {
        Ok(proj) => (StatusCode::CREATED, Json(json!(proj))).into_response(),
        Err(err) => project_error_response(err).into_response(),
    }
}

/// GET /api/timekeeping/projects/:id
pub async fn get_project(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Query(q): Query<AppIdQuery>,
) -> impl IntoResponse {
    match ProjectRepo::find_by_id(&state.pool, id, &q.app_id).await {
        Ok(Some(proj)) => (StatusCode::OK, Json(json!(proj))).into_response(),
        Ok(None) => project_error_response(ProjectError::NotFound).into_response(),
        Err(err) => project_error_response(err).into_response(),
    }
}

/// GET /api/timekeeping/projects
pub async fn list_projects(
    State(state): State<Arc<AppState>>,
    Query(q): Query<AppIdQuery>,
) -> impl IntoResponse {
    match ProjectRepo::list(&state.pool, &q.app_id, q.active_only).await {
        Ok(projects) => (StatusCode::OK, Json(json!(projects))).into_response(),
        Err(err) => project_error_response(err).into_response(),
    }
}

/// PUT /api/timekeeping/projects/:id
pub async fn update_project(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateProjectRequest>,
) -> impl IntoResponse {
    match ProjectRepo::update(&state.pool, id, &req).await {
        Ok(proj) => (StatusCode::OK, Json(json!(proj))).into_response(),
        Err(err) => project_error_response(err).into_response(),
    }
}

/// DELETE /api/timekeeping/projects/:id
pub async fn deactivate_project(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Query(q): Query<AppIdQuery>,
) -> impl IntoResponse {
    match ProjectRepo::deactivate(&state.pool, id, &q.app_id).await {
        Ok(proj) => (StatusCode::OK, Json(json!(proj))).into_response(),
        Err(err) => project_error_response(err).into_response(),
    }
}

// ============================================================================
// Task handlers
// ============================================================================

/// POST /api/timekeeping/tasks
pub async fn create_task(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateTaskRequest>,
) -> impl IntoResponse {
    match TaskRepo::create(&state.pool, &req).await {
        Ok(task) => (StatusCode::CREATED, Json(json!(task))).into_response(),
        Err(err) => task_error_response(err).into_response(),
    }
}

/// GET /api/timekeeping/projects/:project_id/tasks
pub async fn list_tasks(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<Uuid>,
    Query(q): Query<TaskListQuery>,
) -> impl IntoResponse {
    match TaskRepo::list_for_project(&state.pool, project_id, &q.app_id, q.active_only).await {
        Ok(tasks) => (StatusCode::OK, Json(json!(tasks))).into_response(),
        Err(err) => task_error_response(err).into_response(),
    }
}

/// GET /api/timekeeping/tasks/:id
pub async fn get_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Query(q): Query<TaskListQuery>,
) -> impl IntoResponse {
    match TaskRepo::find_by_id(&state.pool, id, &q.app_id).await {
        Ok(Some(task)) => (StatusCode::OK, Json(json!(task))).into_response(),
        Ok(None) => task_error_response(TaskError::NotFound).into_response(),
        Err(err) => task_error_response(err).into_response(),
    }
}

/// PUT /api/timekeeping/tasks/:id
pub async fn update_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateTaskRequest>,
) -> impl IntoResponse {
    match TaskRepo::update(&state.pool, id, &req).await {
        Ok(task) => (StatusCode::OK, Json(json!(task))).into_response(),
        Err(err) => task_error_response(err).into_response(),
    }
}

/// DELETE /api/timekeeping/tasks/:id
pub async fn deactivate_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Query(q): Query<TaskListQuery>,
) -> impl IntoResponse {
    match TaskRepo::deactivate(&state.pool, id, &q.app_id).await {
        Ok(task) => (StatusCode::OK, Json(json!(task))).into_response(),
        Err(err) => task_error_response(err).into_response(),
    }
}
