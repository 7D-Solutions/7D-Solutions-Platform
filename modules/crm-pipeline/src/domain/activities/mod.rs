//! Activity bounded context.
//!
//! Activities must reference at least one entity (lead/opportunity/party/party_contact).
//! Completed activities cannot revert.

pub mod repo;
pub mod service;

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum ActivityError {
    #[error("Activity not found: {0}")]
    NotFound(Uuid),
    #[error("Activity must reference at least one entity (lead, opportunity, party, or contact)")]
    NoEntityReference,
    #[error("Activity is already completed")]
    AlreadyCompleted,
    #[error("Validation error: {0}")]
    Validation(String),
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

impl From<ActivityError> for platform_http_contracts::ApiError {
    fn from(err: ActivityError) -> Self {
        match err {
            ActivityError::NotFound(id) => Self::not_found(format!("Activity {} not found", id)),
            ActivityError::NoEntityReference => {
                Self::new(422, "no_entity_reference", "Activity must reference at least one entity")
            }
            ActivityError::AlreadyCompleted => {
                Self::new(422, "already_completed", "Activity is already completed")
            }
            ActivityError::Validation(msg) => Self::new(422, "validation_error", msg),
            ActivityError::Database(e) => {
                tracing::error!("CRM activities DB error: {}", e);
                Self::internal("Internal database error")
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct Activity {
    pub id: Uuid,
    pub tenant_id: String,
    pub activity_type_code: String,
    pub subject: String,
    pub description: Option<String>,
    pub activity_date: NaiveDate,
    pub duration_minutes: Option<i32>,
    pub lead_id: Option<Uuid>,
    pub opportunity_id: Option<Uuid>,
    pub party_id: Option<Uuid>,
    pub party_contact_id: Option<Uuid>,
    pub due_date: Option<NaiveDate>,
    pub is_completed: bool,
    pub completed_at: Option<DateTime<Utc>>,
    pub assigned_to: Option<String>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateActivityRequest {
    pub activity_type_code: String,
    pub subject: String,
    pub description: Option<String>,
    pub activity_date: NaiveDate,
    pub duration_minutes: Option<i32>,
    pub lead_id: Option<Uuid>,
    pub opportunity_id: Option<Uuid>,
    pub party_id: Option<Uuid>,
    pub party_contact_id: Option<Uuid>,
    pub due_date: Option<NaiveDate>,
    pub assigned_to: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateActivityRequest {
    pub subject: Option<String>,
    pub description: Option<String>,
    pub activity_date: Option<NaiveDate>,
    pub duration_minutes: Option<i32>,
    pub due_date: Option<NaiveDate>,
    pub assigned_to: Option<String>,
}

#[derive(Debug, Deserialize, Default, ToSchema)]
pub struct ListActivitiesQuery {
    pub assigned_to: Option<String>,
    pub lead_id: Option<Uuid>,
    pub opportunity_id: Option<Uuid>,
    pub party_id: Option<Uuid>,
    pub include_completed: Option<bool>,
    pub due_before: Option<NaiveDate>,
}
