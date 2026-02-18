//! Export run domain models and request types.
//!
//! Export runs produce deterministic CSV + JSON artifacts from approved
//! timesheet entries. Content hashes guarantee idempotent re-runs.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

// ============================================================================
// Export status enum (mirrors tk_export_status in DB)
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "tk_export_status", rename_all = "lowercase")]
pub enum ExportStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

// ============================================================================
// Domain models
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ExportRun {
    pub id: Uuid,
    pub app_id: String,
    pub export_type: String,
    pub period_start: NaiveDate,
    pub period_end: NaiveDate,
    pub status: ExportStatus,
    pub record_count: Option<i32>,
    pub metadata: Option<serde_json::Value>,
    pub content_hash: Option<String>,
    pub error_message: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// Flattened entry row used for export generation.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ExportEntry {
    pub entry_id: Uuid,
    pub employee_id: Uuid,
    pub employee_name: String,
    pub project_id: Option<Uuid>,
    pub project_name: Option<String>,
    pub task_id: Option<Uuid>,
    pub work_date: NaiveDate,
    pub minutes: i32,
    pub description: Option<String>,
}

/// Generated export artifacts returned to the caller.
#[derive(Debug, Clone, Serialize)]
pub struct ExportArtifact {
    pub run: ExportRun,
    pub csv: String,
    pub json: serde_json::Value,
}

// ============================================================================
// Request types
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct CreateExportRunRequest {
    pub app_id: String,
    pub export_type: String,
    pub period_start: NaiveDate,
    pub period_end: NaiveDate,
}

#[derive(Debug, Deserialize)]
pub struct ListExportRunsQuery {
    pub app_id: String,
    pub export_type: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GetExportRunQuery {
    pub app_id: String,
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug, Error)]
pub enum ExportError {
    #[error("Export run not found")]
    NotFound,

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("No approved entries found for this period")]
    NoApprovedEntries,

    #[error("Idempotent replay: export with same content already exists")]
    IdempotentReplay { run_id: Uuid },

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
    fn export_status_serde_roundtrip() {
        for status in [
            ExportStatus::Pending,
            ExportStatus::InProgress,
            ExportStatus::Completed,
            ExportStatus::Failed,
        ] {
            let json = serde_json::to_string(&status).unwrap();
            let back: ExportStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(back, status);
        }
    }

    #[test]
    fn create_request_deserialize() {
        let json = r#"{
            "app_id": "acme",
            "export_type": "payroll",
            "period_start": "2026-02-01",
            "period_end": "2026-02-14"
        }"#;
        let req: CreateExportRunRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.app_id, "acme");
        assert_eq!(req.export_type, "payroll");
    }
}
