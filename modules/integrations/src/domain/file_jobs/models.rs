//! File import/export job domain models.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

/// Valid job statuses.
pub const STATUS_CREATED: &str = "created";
pub const STATUS_PROCESSING: &str = "processing";
pub const STATUS_COMPLETED: &str = "completed";
pub const STATUS_FAILED: &str = "failed";

/// A durable file import/export job record.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct FileJob {
    pub id: Uuid,
    pub tenant_id: String,
    pub file_ref: String,
    pub parser_type: String,
    pub status: String,
    pub error_details: Option<String>,
    pub idempotency_key: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Request to create a new file job.
#[derive(Debug, Clone)]
pub struct CreateFileJobRequest {
    pub tenant_id: String,
    pub file_ref: String,
    pub parser_type: String,
    pub idempotency_key: Option<String>,
}

/// Request to transition a job to a new status.
#[derive(Debug, Clone)]
pub struct TransitionFileJobRequest {
    pub job_id: Uuid,
    pub tenant_id: String,
    pub new_status: String,
    pub error_details: Option<String>,
}

#[derive(Debug, Error)]
pub enum FileJobError {
    #[error("File job not found")]
    NotFound,

    #[error("Validation failed: {0}")]
    Validation(String),

    #[error("Invalid status transition from '{from}' to '{to}'")]
    InvalidTransition { from: String, to: String },

    #[error("Duplicate idempotency key")]
    DuplicateIdempotencyKey,

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}
