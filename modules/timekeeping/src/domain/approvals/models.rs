//! Approval workflow domain models and request types.
//!
//! State machine: draft → submitted → approved / rejected → (recall → draft)
//! When approved, the period is locked — entries guards enforce this.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use utoipa::ToSchema;
use uuid::Uuid;

// ============================================================================
// Approval status enum (mirrors tk_approval_status in DB)
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::Type, ToSchema)]
#[sqlx(type_name = "tk_approval_status", rename_all = "lowercase")]
pub enum ApprovalStatus {
    Draft,
    Submitted,
    Approved,
    Rejected,
}

// ============================================================================
// Domain models
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct ApprovalRequest {
    pub id: Uuid,
    pub app_id: String,
    pub employee_id: Uuid,
    pub period_start: NaiveDate,
    pub period_end: NaiveDate,
    pub status: ApprovalStatus,
    pub total_minutes: i32,
    pub submitted_at: Option<DateTime<Utc>>,
    pub reviewed_at: Option<DateTime<Utc>>,
    pub reviewer_id: Option<Uuid>,
    pub reviewer_notes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct ApprovalAction {
    pub id: i64,
    pub approval_id: Uuid,
    pub action: String,
    pub actor_id: Uuid,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
}

// ============================================================================
// Request types
// ============================================================================

#[derive(Debug, Deserialize, ToSchema)]
pub struct SubmitApprovalRequest {
    pub app_id: String,
    pub employee_id: Uuid,
    pub period_start: NaiveDate,
    pub period_end: NaiveDate,
    pub actor_id: Uuid,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ReviewApprovalRequest {
    pub app_id: String,
    pub approval_id: Uuid,
    pub actor_id: Uuid,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RecallApprovalRequest {
    pub app_id: String,
    pub approval_id: Uuid,
    pub actor_id: Uuid,
    pub notes: Option<String>,
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug, Error)]
pub enum ApprovalError {
    #[error("Approval request not found")]
    NotFound,

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Invalid status transition from {from} to {to}")]
    InvalidTransition { from: String, to: String },

    #[error("Duplicate approval request for this period")]
    Duplicate,

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_status_serde_roundtrip() -> Result<(), Box<dyn std::error::Error>> {
        for status in [
            ApprovalStatus::Draft,
            ApprovalStatus::Submitted,
            ApprovalStatus::Approved,
            ApprovalStatus::Rejected,
        ] {
            let json = serde_json::to_string(&status)?;
            let back: ApprovalStatus = serde_json::from_str(&json)?;
            assert_eq!(back, status);
        }
        Ok(())
    }

    #[test]
    fn submit_request_deserialize() -> Result<(), Box<dyn std::error::Error>> {
        let json = r#"{
            "app_id": "acme",
            "employee_id": "00000000-0000-0000-0000-000000000001",
            "period_start": "2026-02-01",
            "period_end": "2026-02-07",
            "actor_id": "00000000-0000-0000-0000-000000000001"
        }"#;
        let req: SubmitApprovalRequest = serde_json::from_str(json)?;
        assert_eq!(req.app_id, "acme");
        Ok(())
    }
}
