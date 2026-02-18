//! Project and Task repository — CRUD operations against tk_projects and tk_tasks.

use sqlx::PgPool;
use uuid::Uuid;

use super::models::{
    CreateProjectRequest, CreateTaskRequest, Project, ProjectError, Task, TaskError,
    UpdateProjectRequest, UpdateTaskRequest,
};

// ============================================================================
// Project repository
// ============================================================================

pub struct ProjectRepo;

impl ProjectRepo {
    /// Create a new project. Returns DuplicateCode on (app_id, project_code) violation.
    pub async fn create(
        pool: &PgPool,
        req: &CreateProjectRequest,
    ) -> Result<Project, ProjectError> {
        req.validate()?;

        let billable = req.billable.unwrap_or(false);

        sqlx::query_as::<_, Project>(
            r#"
            INSERT INTO tk_projects
                (app_id, project_code, name, description, billable, gl_account_ref)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING *
            "#,
        )
        .bind(&req.app_id)
        .bind(req.project_code.trim())
        .bind(req.name.trim())
        .bind(req.description.as_deref())
        .bind(billable)
        .bind(req.gl_account_ref.as_deref())
        .fetch_one(pool)
        .await
        .map_err(|e| {
            if let sqlx::Error::Database(ref dbe) = e {
                if dbe.code().as_deref() == Some("23505") {
                    return ProjectError::DuplicateCode(
                        req.project_code.clone(),
                        req.app_id.clone(),
                    );
                }
            }
            ProjectError::Database(e)
        })
    }

    /// Update mutable project fields.
    pub async fn update(
        pool: &PgPool,
        id: Uuid,
        req: &UpdateProjectRequest,
    ) -> Result<Project, ProjectError> {
        req.validate()?;

        sqlx::query_as::<_, Project>(
            r#"
            UPDATE tk_projects
            SET
                name           = COALESCE($3, name),
                description    = CASE WHEN $4::TEXT IS NOT NULL THEN $4 ELSE description END,
                billable       = COALESCE($5, billable),
                gl_account_ref = CASE WHEN $6::TEXT IS NOT NULL THEN $6 ELSE gl_account_ref END,
                updated_at     = NOW()
            WHERE id = $1 AND app_id = $2
            RETURNING *
            "#,
        )
        .bind(id)
        .bind(&req.app_id)
        .bind(req.name.as_deref())
        .bind(req.description.as_deref())
        .bind(req.billable)
        .bind(req.gl_account_ref.as_deref())
        .fetch_optional(pool)
        .await?
        .ok_or(ProjectError::NotFound)
    }

    /// Deactivate a project (soft delete). Idempotent.
    pub async fn deactivate(
        pool: &PgPool,
        id: Uuid,
        app_id: &str,
    ) -> Result<Project, ProjectError> {
        sqlx::query_as::<_, Project>(
            r#"
            UPDATE tk_projects
            SET active = FALSE, updated_at = NOW()
            WHERE id = $1 AND app_id = $2
            RETURNING *
            "#,
        )
        .bind(id)
        .bind(app_id)
        .fetch_optional(pool)
        .await?
        .ok_or(ProjectError::NotFound)
    }

    /// Fetch a project by id, scoped to app_id.
    pub async fn find_by_id(
        pool: &PgPool,
        id: Uuid,
        app_id: &str,
    ) -> Result<Option<Project>, ProjectError> {
        sqlx::query_as::<_, Project>(
            "SELECT * FROM tk_projects WHERE id = $1 AND app_id = $2",
        )
        .bind(id)
        .bind(app_id)
        .fetch_optional(pool)
        .await
        .map_err(ProjectError::Database)
    }

    /// List projects for a tenant.
    pub async fn list(
        pool: &PgPool,
        app_id: &str,
        active_only: bool,
    ) -> Result<Vec<Project>, ProjectError> {
        if active_only {
            sqlx::query_as::<_, Project>(
                r#"
                SELECT * FROM tk_projects
                WHERE app_id = $1 AND active = TRUE
                ORDER BY project_code
                "#,
            )
            .bind(app_id)
            .fetch_all(pool)
            .await
            .map_err(ProjectError::Database)
        } else {
            sqlx::query_as::<_, Project>(
                r#"
                SELECT * FROM tk_projects
                WHERE app_id = $1
                ORDER BY project_code
                "#,
            )
            .bind(app_id)
            .fetch_all(pool)
            .await
            .map_err(ProjectError::Database)
        }
    }
}

// ============================================================================
// Task repository
// ============================================================================

pub struct TaskRepo;

impl TaskRepo {
    /// Create a new task under a project.
    /// Returns TaskError::ProjectNotFound if the parent project doesn't exist.
    /// Returns TaskError::DuplicateCode on (project_id, task_code) violation.
    pub async fn create(pool: &PgPool, req: &CreateTaskRequest) -> Result<Task, TaskError> {
        req.validate()?;

        // Verify parent project exists and belongs to the same app
        let project_exists = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM tk_projects WHERE id = $1 AND app_id = $2)",
        )
        .bind(req.project_id)
        .bind(&req.app_id)
        .fetch_one(pool)
        .await
        .map_err(TaskError::Database)?;

        if !project_exists {
            return Err(TaskError::ProjectNotFound);
        }

        sqlx::query_as::<_, Task>(
            r#"
            INSERT INTO tk_tasks (app_id, project_id, task_code, name)
            VALUES ($1, $2, $3, $4)
            RETURNING *
            "#,
        )
        .bind(&req.app_id)
        .bind(req.project_id)
        .bind(req.task_code.trim())
        .bind(req.name.trim())
        .fetch_one(pool)
        .await
        .map_err(|e| {
            if let sqlx::Error::Database(ref dbe) = e {
                if dbe.code().as_deref() == Some("23505") {
                    return TaskError::DuplicateCode(
                        req.task_code.clone(),
                        req.project_id,
                    );
                }
            }
            TaskError::Database(e)
        })
    }

    /// Update task name.
    pub async fn update(
        pool: &PgPool,
        id: Uuid,
        req: &UpdateTaskRequest,
    ) -> Result<Task, TaskError> {
        req.validate()?;

        sqlx::query_as::<_, Task>(
            r#"
            UPDATE tk_tasks
            SET name = COALESCE($3, name), updated_at = NOW()
            WHERE id = $1 AND app_id = $2
            RETURNING *
            "#,
        )
        .bind(id)
        .bind(&req.app_id)
        .bind(req.name.as_deref())
        .fetch_optional(pool)
        .await?
        .ok_or(TaskError::NotFound)
    }

    /// Deactivate a task (soft delete). Idempotent.
    pub async fn deactivate(pool: &PgPool, id: Uuid, app_id: &str) -> Result<Task, TaskError> {
        sqlx::query_as::<_, Task>(
            r#"
            UPDATE tk_tasks
            SET active = FALSE, updated_at = NOW()
            WHERE id = $1 AND app_id = $2
            RETURNING *
            "#,
        )
        .bind(id)
        .bind(app_id)
        .fetch_optional(pool)
        .await?
        .ok_or(TaskError::NotFound)
    }

    /// Fetch a task by id, scoped to app_id.
    pub async fn find_by_id(
        pool: &PgPool,
        id: Uuid,
        app_id: &str,
    ) -> Result<Option<Task>, TaskError> {
        sqlx::query_as::<_, Task>(
            "SELECT * FROM tk_tasks WHERE id = $1 AND app_id = $2",
        )
        .bind(id)
        .bind(app_id)
        .fetch_optional(pool)
        .await
        .map_err(TaskError::Database)
    }

    /// List tasks for a project.
    pub async fn list_for_project(
        pool: &PgPool,
        project_id: Uuid,
        app_id: &str,
        active_only: bool,
    ) -> Result<Vec<Task>, TaskError> {
        if active_only {
            sqlx::query_as::<_, Task>(
                r#"
                SELECT * FROM tk_tasks
                WHERE project_id = $1 AND app_id = $2 AND active = TRUE
                ORDER BY task_code
                "#,
            )
            .bind(project_id)
            .bind(app_id)
            .fetch_all(pool)
            .await
            .map_err(TaskError::Database)
        } else {
            sqlx::query_as::<_, Task>(
                r#"
                SELECT * FROM tk_tasks
                WHERE project_id = $1 AND app_id = $2
                ORDER BY task_code
                "#,
            )
            .bind(project_id)
            .bind(app_id)
            .fetch_all(pool)
            .await
            .map_err(TaskError::Database)
        }
    }
}
