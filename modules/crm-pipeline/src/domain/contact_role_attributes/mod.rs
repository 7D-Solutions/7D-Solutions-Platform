//! CRM-specific attributes layered on top of Party contacts.
//!
//! Does not own the contact record — Party does.
//! party.contact.deactivated events set is_active = false.

pub mod repo;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum ContactRoleError {
    #[error("Contact role attributes not found for contact {0}")]
    NotFound(Uuid),
    #[error("Validation error: {0}")]
    Validation(String),
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

impl From<ContactRoleError> for platform_http_contracts::ApiError {
    fn from(err: ContactRoleError) -> Self {
        match err {
            ContactRoleError::NotFound(id) => Self::not_found(format!("Contact role attributes for {} not found", id)),
            ContactRoleError::Validation(msg) => Self::new(422, "validation_error", msg),
            ContactRoleError::Database(e) => {
                tracing::error!("CRM contact_role_attributes DB error: {}", e);
                Self::internal("Internal database error")
            }
        }
    }
}

/// Canonical sales roles a contact can play.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SalesRole {
    DecisionMaker,
    Champion,
    Influencer,
    User,
    Blocker,
    Unknown,
}

impl SalesRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            SalesRole::DecisionMaker => "decision_maker",
            SalesRole::Champion => "champion",
            SalesRole::Influencer => "influencer",
            SalesRole::User => "user",
            SalesRole::Blocker => "blocker",
            SalesRole::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct ContactRoleAttributes {
    pub id: Uuid,
    pub tenant_id: String,
    pub party_contact_id: Uuid,
    pub sales_role: String,
    pub is_primary_buyer: bool,
    pub is_economic_buyer: bool,
    pub is_active: bool,
    pub notes: Option<String>,
    pub updated_by: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpsertContactRoleRequest {
    pub sales_role: Option<String>,
    pub is_primary_buyer: Option<bool>,
    pub is_economic_buyer: Option<bool>,
    pub notes: Option<String>,
}
