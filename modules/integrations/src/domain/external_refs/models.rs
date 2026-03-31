//! External reference models and request types.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use utoipa::ToSchema;

/// A single external ID mapping: internal entity ↔ external system identifier.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct ExternalRef {
    pub id: i64,
    pub app_id: String,
    pub entity_type: String,
    pub entity_id: String,
    pub system: String,
    pub external_id: String,
    pub label: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Create a new external ref (or upsert if the same system+external_id exists).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateExternalRefRequest {
    /// Internal entity type (e.g. "invoice", "customer", "order").
    pub entity_type: String,
    /// Internal entity ID (UUID or opaque string).
    pub entity_id: String,
    /// External system name (e.g. "stripe", "quickbooks", "salesforce").
    pub system: String,
    /// The identifier in the external system.
    pub external_id: String,
    /// Optional human-readable label.
    pub label: Option<String>,
    /// Optional arbitrary metadata.
    pub metadata: Option<serde_json::Value>,
}

/// Update mutable fields on an existing external ref.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateExternalRefRequest {
    pub label: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Error)]
pub enum ExternalRefError {
    #[error("External ref {0} not found")]
    NotFound(i64),
    #[error("Conflict: {0}")]
    Conflict(String),
    #[error("Validation error: {0}")]
    Validation(String),
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}
