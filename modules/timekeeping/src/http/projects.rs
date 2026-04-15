//! Project and Task HTTP handlers — CRUD endpoints for project/task catalog.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use platform_http_contracts::{ApiError, PaginatedResponse};
use security::VerifiedClaims;
use serde::Serialize;
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    domain::projects::{
        models::{
            CreateProjectRequest, CreateTaskRequest, Project, ProjectError, Task, TaskError,
            UpdateProjectRequest, UpdateTaskRequest,
        },
        service::{ProjectRepo, TaskRepo},
    },
    AppState,
};
use platform_sdk::extract_tenant;

// ============================================================================
// Error mapping
// ============================================================================

fn map_project_error(err: ProjectError) -> ApiError {
    match err {
        ProjectError::NotFound => ApiError::not_found("Project not found"),
        ProjectError::DuplicateCode(code, app) => ApiError::conflict(format!(
            "Project code '{}' already exists for app '{}'",
            code, app
        )),
        ProjectError::Validation(msg) => ApiError::new(422, "validation_error", msg),
        ProjectError::Database(e) => ApiError::internal(e.to_string()),
    }
}

fn map_task_error(err: TaskError) -> ApiError {
    match err {
        TaskError::NotFound => ApiError::not_found("Task not found"),
        TaskError::ProjectNotFound => ApiError::not_found("Parent project not found"),
        TaskError::DuplicateCode(code, project_id) => ApiError::conflict(format!(
            "Task code '{}' already exists for project '{}'",
            code, project_id
        )),
        TaskError::Validation(msg) => ApiError::new(422, "validation_error", msg),
        TaskError::Database(e) => ApiError::internal(e.to_string()),
    }
}

// ============================================================================
// Sub-collection wrapper
// ============================================================================

#[derive(Debug, Serialize)]
pub struct DataWrapper<T: Serialize> {
    pub data: Vec<T>,
}

// ============================================================================
// Project handlers
// ============================================================================

#[utoipa::path(
    post,
    path = "/api/timekeeping/projects",
    request_body = CreateProjectRequest,
    responses(
        (status = 201, description = "Project created", body = Project),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 409, description = "Duplicate project code", body = ApiError),
        (status = 422, description = "Validation error", body = ApiError),
    ),
    security(("bearer" = [])),
    tag = "Projects",
)]
pub async fn create_project(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<CreateProjectRequest>,
) -> Result<(StatusCode, Json<Project>), ApiError> {
    let app_id = extract_tenant(&claims)?;
    req.app_id = app_id;
    let proj = ProjectRepo::create(&state.pool, &req)
        .await
        .map_err(map_project_error)?;
    Ok((StatusCode::CREATED, Json(proj)))
}

#[utoipa::path(
    get,
    path = "/api/timekeeping/projects/{id}",
    params(("id" = Uuid, Path, description = "Project UUID")),
    responses(
        (status = 200, description = "Project found", body = Project),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = [])),
    tag = "Projects",
)]
pub async fn get_project(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Project>, ApiError> {
    let app_id = extract_tenant(&claims)?;
    let proj = ProjectRepo::find_by_id(&state.pool, id, &app_id)
        .await
        .map_err(map_project_error)?
        .ok_or_else(|| ApiError::not_found("Project not found"))?;
    Ok(Json(proj))
}

#[utoipa::path(
    get,
    path = "/api/timekeeping/projects",
    responses(
        (status = 200, description = "Project list", body = Vec<Project>),
        (status = 401, description = "Unauthorized", body = ApiError),
    ),
    security(("bearer" = [])),
    tag = "Projects",
)]
pub async fn list_projects(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
) -> Result<Json<PaginatedResponse<Project>>, ApiError> {
    let app_id = extract_tenant(&claims)?;
    let projects = ProjectRepo::list(&state.pool, &app_id, true)
        .await
        .map_err(map_project_error)?;
    let total = projects.len() as i64;
    Ok(Json(PaginatedResponse::new(projects, 1, total, total)))
}

#[utoipa::path(
    put,
    path = "/api/timekeeping/projects/{id}",
    params(("id" = Uuid, Path, description = "Project UUID")),
    request_body = UpdateProjectRequest,
    responses(
        (status = 200, description = "Project updated", body = Project),
        (status = 404, description = "Not found", body = ApiError),
        (status = 422, description = "Validation error", body = ApiError),
    ),
    security(("bearer" = [])),
    tag = "Projects",
)]
pub async fn update_project(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateProjectRequest>,
) -> Result<Json<Project>, ApiError> {
    let proj = ProjectRepo::update(&state.pool, id, &req)
        .await
        .map_err(map_project_error)?;
    Ok(Json(proj))
}

#[utoipa::path(
    delete,
    path = "/api/timekeeping/projects/{id}",
    params(("id" = Uuid, Path, description = "Project UUID")),
    responses(
        (status = 200, description = "Project deactivated", body = Project),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = [])),
    tag = "Projects",
)]
pub async fn deactivate_project(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Project>, ApiError> {
    let app_id = extract_tenant(&claims)?;
    let proj = ProjectRepo::deactivate(&state.pool, id, &app_id)
        .await
        .map_err(map_project_error)?;
    Ok(Json(proj))
}

// ============================================================================
// Task handlers
// ============================================================================

#[utoipa::path(
    post,
    path = "/api/timekeeping/tasks",
    request_body = CreateTaskRequest,
    responses(
        (status = 201, description = "Task created", body = Task),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 409, description = "Duplicate task code", body = ApiError),
        (status = 422, description = "Validation error", body = ApiError),
    ),
    security(("bearer" = [])),
    tag = "Tasks",
)]
pub async fn create_task(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<CreateTaskRequest>,
) -> Result<(StatusCode, Json<Task>), ApiError> {
    let app_id = extract_tenant(&claims)?;
    req.app_id = app_id;
    let task = TaskRepo::create(&state.pool, &req)
        .await
        .map_err(map_task_error)?;
    Ok((StatusCode::CREATED, Json(task)))
}

#[utoipa::path(
    get,
    path = "/api/timekeeping/projects/{project_id}/tasks",
    params(("project_id" = Uuid, Path, description = "Parent project UUID")),
    responses(
        (status = 200, description = "Task list for project", body = PaginatedResponse<Task>),
        (status = 401, description = "Unauthorized", body = ApiError),
    ),
    security(("bearer" = [])),
    tag = "Tasks",
)]
pub async fn list_tasks(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(project_id): Path<Uuid>,
) -> Result<Json<PaginatedResponse<Task>>, ApiError> {
    let app_id = extract_tenant(&claims)?;
    let tasks = TaskRepo::list_for_project(&state.pool, project_id, &app_id, true)
        .await
        .map_err(map_task_error)?;
    let total = tasks.len() as i64;
    Ok(Json(PaginatedResponse::new(tasks, 1, total, total)))
}

#[utoipa::path(
    get,
    path = "/api/timekeeping/tasks/{id}",
    params(("id" = Uuid, Path, description = "Task UUID")),
    responses(
        (status = 200, description = "Task found", body = Task),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = [])),
    tag = "Tasks",
)]
pub async fn get_task(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Task>, ApiError> {
    let app_id = extract_tenant(&claims)?;
    let task = TaskRepo::find_by_id(&state.pool, id, &app_id)
        .await
        .map_err(map_task_error)?
        .ok_or_else(|| ApiError::not_found("Task not found"))?;
    Ok(Json(task))
}

#[utoipa::path(
    put,
    path = "/api/timekeeping/tasks/{id}",
    params(("id" = Uuid, Path, description = "Task UUID")),
    request_body = UpdateTaskRequest,
    responses(
        (status = 200, description = "Task updated", body = Task),
        (status = 404, description = "Not found", body = ApiError),
        (status = 422, description = "Validation error", body = ApiError),
    ),
    security(("bearer" = [])),
    tag = "Tasks",
)]
pub async fn update_task(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateTaskRequest>,
) -> Result<Json<Task>, ApiError> {
    let task = TaskRepo::update(&state.pool, id, &req)
        .await
        .map_err(map_task_error)?;
    Ok(Json(task))
}

#[utoipa::path(
    delete,
    path = "/api/timekeeping/tasks/{id}",
    params(("id" = Uuid, Path, description = "Task UUID")),
    responses(
        (status = 200, description = "Task deactivated", body = Task),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = [])),
    tag = "Tasks",
)]
pub async fn deactivate_task(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Task>, ApiError> {
    let app_id = extract_tenant(&claims)?;
    let task = TaskRepo::deactivate(&state.pool, id, &app_id)
        .await
        .map_err(map_task_error)?;
    Ok(Json(task))
}
