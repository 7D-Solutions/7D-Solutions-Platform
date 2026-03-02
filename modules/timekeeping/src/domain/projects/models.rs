//! Project and Task domain models.
//!
//! Invariants:
//! - project_code is unique per app_id (DB constraint)
//! - task_code is unique per project_id (DB constraint)
//! - gl_account_ref is a soft reference to GL accounts (not enforced via FK)
//! - Deactivate is idempotent

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

// ============================================================================
// Domain models
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Project {
    pub id: Uuid,
    pub app_id: String,
    pub project_code: String,
    pub name: String,
    pub description: Option<String>,
    pub billable: bool,
    pub gl_account_ref: Option<String>,
    pub active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Task {
    pub id: Uuid,
    pub app_id: String,
    pub project_id: Uuid,
    pub task_code: String,
    pub name: String,
    pub active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ============================================================================
// Request types
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct CreateProjectRequest {
    pub app_id: String,
    pub project_code: String,
    pub name: String,
    pub description: Option<String>,
    pub billable: Option<bool>,
    pub gl_account_ref: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateProjectRequest {
    pub app_id: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub billable: Option<bool>,
    pub gl_account_ref: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateTaskRequest {
    pub app_id: String,
    pub project_id: Uuid,
    pub task_code: String,
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateTaskRequest {
    pub app_id: String,
    pub name: Option<String>,
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug, Error)]
pub enum ProjectError {
    #[error("Project code '{0}' already exists for app '{1}'")]
    DuplicateCode(String, String),

    #[error("Project not found")]
    NotFound,

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

#[derive(Debug, Error)]
pub enum TaskError {
    #[error("Task code '{0}' already exists for project '{1}'")]
    DuplicateCode(String, Uuid),

    #[error("Task not found")]
    NotFound,

    #[error("Project not found")]
    ProjectNotFound,

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ============================================================================
// Validation
// ============================================================================

fn require_non_empty(value: &str, field: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("{} must not be empty", field));
    }
    Ok(())
}

impl CreateProjectRequest {
    pub fn validate(&self) -> Result<(), ProjectError> {
        require_non_empty(&self.app_id, "app_id").map_err(ProjectError::Validation)?;
        require_non_empty(&self.project_code, "project_code").map_err(ProjectError::Validation)?;
        require_non_empty(&self.name, "name").map_err(ProjectError::Validation)?;
        Ok(())
    }
}

impl UpdateProjectRequest {
    pub fn validate(&self) -> Result<(), ProjectError> {
        require_non_empty(&self.app_id, "app_id").map_err(ProjectError::Validation)?;
        if let Some(ref name) = self.name {
            require_non_empty(name, "name").map_err(ProjectError::Validation)?;
        }
        Ok(())
    }
}

impl CreateTaskRequest {
    pub fn validate(&self) -> Result<(), TaskError> {
        require_non_empty(&self.app_id, "app_id").map_err(TaskError::Validation)?;
        require_non_empty(&self.task_code, "task_code").map_err(TaskError::Validation)?;
        require_non_empty(&self.name, "name").map_err(TaskError::Validation)?;
        Ok(())
    }
}

impl UpdateTaskRequest {
    pub fn validate(&self) -> Result<(), TaskError> {
        require_non_empty(&self.app_id, "app_id").map_err(TaskError::Validation)?;
        if let Some(ref name) = self.name {
            require_non_empty(name, "name").map_err(TaskError::Validation)?;
        }
        Ok(())
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_project_create() -> CreateProjectRequest {
        CreateProjectRequest {
            app_id: "acme".to_string(),
            project_code: "PROJ-001".to_string(),
            name: "Website Redesign".to_string(),
            description: Some("Full redesign".to_string()),
            billable: Some(true),
            gl_account_ref: Some("4000".to_string()),
        }
    }

    #[test]
    fn project_create_valid() {
        assert!(valid_project_create().validate().is_ok());
    }

    #[test]
    fn project_create_empty_code_rejected() {
        let mut r = valid_project_create();
        r.project_code = "  ".to_string();
        assert!(matches!(r.validate(), Err(ProjectError::Validation(_))));
    }

    #[test]
    fn project_create_empty_name_rejected() {
        let mut r = valid_project_create();
        r.name = "".to_string();
        assert!(matches!(r.validate(), Err(ProjectError::Validation(_))));
    }

    #[test]
    fn project_update_valid_all_none() {
        let r = UpdateProjectRequest {
            app_id: "acme".to_string(),
            name: None,
            description: None,
            billable: None,
            gl_account_ref: None,
        };
        assert!(r.validate().is_ok());
    }

    fn valid_task_create() -> CreateTaskRequest {
        CreateTaskRequest {
            app_id: "acme".to_string(),
            project_id: Uuid::new_v4(),
            task_code: "TASK-001".to_string(),
            name: "Design mockups".to_string(),
        }
    }

    #[test]
    fn task_create_valid() {
        assert!(valid_task_create().validate().is_ok());
    }

    #[test]
    fn task_create_empty_code_rejected() {
        let mut r = valid_task_create();
        r.task_code = "".to_string();
        assert!(matches!(r.validate(), Err(TaskError::Validation(_))));
    }

    #[test]
    fn task_create_empty_name_rejected() {
        let mut r = valid_task_create();
        r.name = " ".to_string();
        assert!(matches!(r.validate(), Err(TaskError::Validation(_))));
    }
}
