//! Allocation domain models — planned hours and actual-time rollups.
//!
//! Allocations represent planned minutes-per-week for an employee on a project/task.
//! Rollups aggregate actual minutes from tk_timesheet_entries (is_current = TRUE,
//! entry_type != 'void') grouped by employee, project, task, and date range.
//! All math uses integer minutes — no floating-point.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

// ============================================================================
// Domain model (mirrors tk_allocations)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Allocation {
    pub id: Uuid,
    pub app_id: String,
    pub employee_id: Uuid,
    pub project_id: Uuid,
    pub task_id: Option<Uuid>,
    pub allocated_minutes_per_week: i32,
    pub effective_from: NaiveDate,
    pub effective_to: Option<NaiveDate>,
    pub active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ============================================================================
// Request types
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct CreateAllocationRequest {
    pub app_id: String,
    pub employee_id: Uuid,
    pub project_id: Uuid,
    pub task_id: Option<Uuid>,
    pub allocated_minutes_per_week: i32,
    pub effective_from: NaiveDate,
    pub effective_to: Option<NaiveDate>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateAllocationRequest {
    pub app_id: String,
    pub allocated_minutes_per_week: Option<i32>,
    pub effective_to: Option<NaiveDate>,
}

// ============================================================================
// Rollup response types
// ============================================================================

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct ProjectRollup {
    pub project_id: Uuid,
    pub project_name: String,
    pub total_minutes: i64,
    pub entry_count: i64,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct EmployeeRollup {
    pub employee_id: Uuid,
    pub first_name: String,
    pub last_name: String,
    pub total_minutes: i64,
    pub entry_count: i64,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct TaskRollup {
    pub project_id: Uuid,
    pub task_id: Uuid,
    pub task_name: String,
    pub total_minutes: i64,
    pub entry_count: i64,
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug, Error)]
pub enum AllocationError {
    #[error("Allocation not found")]
    NotFound,

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ============================================================================
// Validation
// ============================================================================

impl CreateAllocationRequest {
    pub fn validate(&self) -> Result<(), AllocationError> {
        if self.app_id.trim().is_empty() {
            return Err(AllocationError::Validation(
                "app_id must not be empty".into(),
            ));
        }
        if self.allocated_minutes_per_week <= 0 {
            return Err(AllocationError::Validation(
                "allocated_minutes_per_week must be positive".into(),
            ));
        }
        if let Some(to) = self.effective_to {
            if to < self.effective_from {
                return Err(AllocationError::Validation(
                    "effective_to must be >= effective_from".into(),
                ));
            }
        }
        Ok(())
    }
}

impl UpdateAllocationRequest {
    pub fn validate(&self) -> Result<(), AllocationError> {
        if self.app_id.trim().is_empty() {
            return Err(AllocationError::Validation(
                "app_id must not be empty".into(),
            ));
        }
        if let Some(mins) = self.allocated_minutes_per_week {
            if mins <= 0 {
                return Err(AllocationError::Validation(
                    "allocated_minutes_per_week must be positive".into(),
                ));
            }
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

    fn valid_create() -> CreateAllocationRequest {
        CreateAllocationRequest {
            app_id: "acme".to_string(),
            employee_id: Uuid::new_v4(),
            project_id: Uuid::new_v4(),
            task_id: None,
            allocated_minutes_per_week: 2400,
            effective_from: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            effective_to: Some(NaiveDate::from_ymd_opt(2026, 6, 30).unwrap()),
        }
    }

    #[test]
    fn create_valid() {
        assert!(valid_create().validate().is_ok());
    }

    #[test]
    fn create_empty_app_id() {
        let mut r = valid_create();
        r.app_id = "  ".into();
        assert!(matches!(r.validate(), Err(AllocationError::Validation(_))));
    }

    #[test]
    fn create_zero_minutes() {
        let mut r = valid_create();
        r.allocated_minutes_per_week = 0;
        assert!(matches!(r.validate(), Err(AllocationError::Validation(_))));
    }

    #[test]
    fn create_negative_minutes() {
        let mut r = valid_create();
        r.allocated_minutes_per_week = -10;
        assert!(matches!(r.validate(), Err(AllocationError::Validation(_))));
    }

    #[test]
    fn create_end_before_start() {
        let mut r = valid_create();
        r.effective_to = Some(NaiveDate::from_ymd_opt(2025, 12, 31).unwrap());
        assert!(matches!(r.validate(), Err(AllocationError::Validation(_))));
    }

    #[test]
    fn create_no_end_date_ok() {
        let mut r = valid_create();
        r.effective_to = None;
        assert!(r.validate().is_ok());
    }

    #[test]
    fn update_valid() {
        let r = UpdateAllocationRequest {
            app_id: "acme".into(),
            allocated_minutes_per_week: Some(1200),
            effective_to: None,
        };
        assert!(r.validate().is_ok());
    }

    #[test]
    fn update_zero_minutes() {
        let r = UpdateAllocationRequest {
            app_id: "acme".into(),
            allocated_minutes_per_week: Some(0),
            effective_to: None,
        };
        assert!(matches!(r.validate(), Err(AllocationError::Validation(_))));
    }
}
