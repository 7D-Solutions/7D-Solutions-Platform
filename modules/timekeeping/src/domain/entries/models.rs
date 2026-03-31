//! Timesheet entry domain model and request types.
//!
//! Append-only strategy: entries are never updated or deleted.
//! Corrections insert a new row with the same entry_id and incremented version.
//! Only the latest version (is_current = TRUE) counts toward totals.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use utoipa::ToSchema;
use uuid::Uuid;

// ============================================================================
// Entry type enum (mirrors tk_entry_type in DB)
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::Type, ToSchema)]
#[sqlx(type_name = "tk_entry_type", rename_all = "lowercase")]
pub enum EntryType {
    Original,
    Correction,
    Void,
}

// ============================================================================
// Domain model
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct TimesheetEntry {
    pub id: i64,
    pub entry_id: Uuid,
    pub version: i32,
    pub app_id: String,
    pub employee_id: Uuid,
    pub project_id: Option<Uuid>,
    pub task_id: Option<Uuid>,
    pub work_date: NaiveDate,
    pub minutes: i32,
    pub description: Option<String>,
    pub entry_type: EntryType,
    pub is_current: bool,
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

// ============================================================================
// Request types
// ============================================================================

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateEntryRequest {
    pub app_id: String,
    pub employee_id: Uuid,
    pub project_id: Option<Uuid>,
    pub task_id: Option<Uuid>,
    pub work_date: NaiveDate,
    pub minutes: i32,
    pub description: Option<String>,
    pub created_by: Option<Uuid>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CorrectEntryRequest {
    pub app_id: String,
    pub entry_id: Uuid,
    pub minutes: i32,
    pub description: Option<String>,
    pub project_id: Option<Uuid>,
    pub task_id: Option<Uuid>,
    pub created_by: Option<Uuid>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct VoidEntryRequest {
    pub app_id: String,
    pub entry_id: Uuid,
    pub created_by: Option<Uuid>,
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug, Error)]
pub enum EntryError {
    #[error("Entry not found")]
    NotFound,

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Period locked: {0}")]
    PeriodLocked(String),

    #[error("Overlap detected: employee already has time for this date/project/task")]
    Overlap,

    #[error("Idempotent replay")]
    IdempotentReplay {
        status_code: u16,
        body: serde_json::Value,
    },

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
    fn entry_type_serde_roundtrip() -> Result<(), Box<dyn std::error::Error>> {
        let orig = EntryType::Original;
        let json = serde_json::to_string(&orig)?;
        assert!(json.contains("original") || json.contains("Original"));
        let back: EntryType = serde_json::from_str(&json)?;
        assert_eq!(back, orig);
        Ok(())
    }

    #[test]
    fn entry_type_correction() -> Result<(), Box<dyn std::error::Error>> {
        let c = EntryType::Correction;
        let json = serde_json::to_string(&c)?;
        let back: EntryType = serde_json::from_str(&json)?;
        assert_eq!(back, EntryType::Correction);
        Ok(())
    }
}
