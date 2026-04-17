//! Activity type configuration (tenant-defined taxonomy).

pub mod repo;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum ActivityTypeError {
    #[error("Activity type not found: {0}")]
    NotFound(String),
    #[error("Duplicate activity type code '{0}'")]
    DuplicateCode(String),
    #[error("Validation error: {0}")]
    Validation(String),
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

impl From<ActivityTypeError> for platform_http_contracts::ApiError {
    fn from(err: ActivityTypeError) -> Self {
        match err {
            ActivityTypeError::NotFound(code) => Self::not_found(format!("Activity type '{}' not found", code)),
            ActivityTypeError::DuplicateCode(code) => Self::conflict(format!("Activity type '{}' already exists", code)),
            ActivityTypeError::Validation(msg) => Self::new(422, "validation_error", msg),
            ActivityTypeError::Database(e) => {
                tracing::error!("CRM activity_types DB error: {}", e);
                Self::internal("Internal database error")
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct ActivityType {
    pub id: Uuid,
    pub tenant_id: String,
    pub activity_type_code: String,
    pub display_label: String,
    pub icon: Option<String>,
    pub active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub updated_by: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateActivityTypeRequest {
    pub activity_type_code: String,
    pub display_label: String,
    pub icon: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateActivityTypeRequest {
    pub display_label: Option<String>,
    pub icon: Option<String>,
    pub active: Option<bool>,
}
