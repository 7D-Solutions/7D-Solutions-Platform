//! Validation guards for timesheet entries.
//!
//! Guards run before mutation. They enforce:
//! - Required field presence (app_id, employee_id, work_date, minutes)
//! - Non-negative duration
//! - Maximum single-entry duration (24h = 1440 minutes)
//! - No duplicate time for same employee + date + project + task (overlap)
//! - Period lock: if approval status is 'approved', reject mutations

use chrono::NaiveDate;
use sqlx::PgPool;
use uuid::Uuid;

use super::models::{CorrectEntryRequest, CreateEntryRequest, EntryError, VoidEntryRequest};

/// Maximum minutes in a single entry (24 hours).
const MAX_ENTRY_MINUTES: i32 = 1440;

// ============================================================================
// Create guards
// ============================================================================

pub fn validate_create(req: &CreateEntryRequest) -> Result<(), EntryError> {
    if req.app_id.trim().is_empty() {
        return Err(EntryError::Validation("app_id must not be empty".into()));
    }
    if req.minutes < 0 {
        return Err(EntryError::Validation(
            "minutes must not be negative".into(),
        ));
    }
    if req.minutes > MAX_ENTRY_MINUTES {
        return Err(EntryError::Validation(format!(
            "minutes exceeds maximum of {}",
            MAX_ENTRY_MINUTES
        )));
    }
    Ok(())
}

// ============================================================================
// Correct guards
// ============================================================================

pub fn validate_correct(req: &CorrectEntryRequest) -> Result<(), EntryError> {
    if req.app_id.trim().is_empty() {
        return Err(EntryError::Validation("app_id must not be empty".into()));
    }
    if req.minutes < 0 {
        return Err(EntryError::Validation(
            "minutes must not be negative".into(),
        ));
    }
    if req.minutes > MAX_ENTRY_MINUTES {
        return Err(EntryError::Validation(format!(
            "minutes exceeds maximum of {}",
            MAX_ENTRY_MINUTES
        )));
    }
    Ok(())
}

// ============================================================================
// Void guards
// ============================================================================

pub fn validate_void(req: &VoidEntryRequest) -> Result<(), EntryError> {
    if req.app_id.trim().is_empty() {
        return Err(EntryError::Validation("app_id must not be empty".into()));
    }
    Ok(())
}

// ============================================================================
// Period lock check
// ============================================================================

/// Check if the period containing `work_date` for the given employee
/// has been approved. If so, mutations are rejected.
pub async fn check_period_lock(
    pool: &PgPool,
    app_id: &str,
    employee_id: Uuid,
    work_date: NaiveDate,
) -> Result<(), EntryError> {
    let locked: Option<(String,)> = sqlx::query_as(
        r#"
        SELECT status::TEXT
        FROM tk_approval_requests
        WHERE app_id = $1
          AND employee_id = $2
          AND period_start <= $3
          AND period_end >= $3
          AND status = 'approved'
        LIMIT 1
        "#,
    )
    .bind(app_id)
    .bind(employee_id)
    .bind(work_date)
    .fetch_optional(pool)
    .await
    .map_err(EntryError::Database)?;

    if locked.is_some() {
        return Err(EntryError::PeriodLocked(format!(
            "Period containing {} for employee {} is approved and locked",
            work_date, employee_id
        )));
    }

    Ok(())
}

// ============================================================================
// Overlap check
// ============================================================================

/// Check if an active (is_current) entry already exists for the same
/// employee + date + project + task. Only blocks creation, not corrections.
pub async fn check_overlap(
    pool: &PgPool,
    app_id: &str,
    employee_id: Uuid,
    work_date: NaiveDate,
    project_id: Option<Uuid>,
    task_id: Option<Uuid>,
) -> Result<(), EntryError> {
    let exists: Option<(i64,)> = sqlx::query_as(
        r#"
        SELECT id FROM tk_timesheet_entries
        WHERE app_id = $1
          AND employee_id = $2
          AND work_date = $3
          AND (project_id IS NOT DISTINCT FROM $4)
          AND (task_id IS NOT DISTINCT FROM $5)
          AND is_current = TRUE
          AND entry_type != 'void'
        LIMIT 1
        "#,
    )
    .bind(app_id)
    .bind(employee_id)
    .bind(work_date)
    .bind(project_id)
    .bind(task_id)
    .fetch_optional(pool)
    .await
    .map_err(EntryError::Database)?;

    if exists.is_some() {
        return Err(EntryError::Overlap);
    }

    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn valid_create() -> CreateEntryRequest {
        CreateEntryRequest {
            app_id: "acme".to_string(),
            employee_id: Uuid::new_v4(),
            project_id: Some(Uuid::new_v4()),
            task_id: None,
            work_date: NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
            minutes: 480,
            description: Some("Worked on feature X".into()),
            created_by: Some(Uuid::new_v4()),
        }
    }

    #[test]
    fn create_valid() {
        assert!(validate_create(&valid_create()).is_ok());
    }

    #[test]
    fn create_empty_app_id() {
        let mut r = valid_create();
        r.app_id = "  ".into();
        assert!(matches!(
            validate_create(&r),
            Err(EntryError::Validation(_))
        ));
    }

    #[test]
    fn create_negative_minutes() {
        let mut r = valid_create();
        r.minutes = -1;
        assert!(matches!(
            validate_create(&r),
            Err(EntryError::Validation(_))
        ));
    }

    #[test]
    fn create_exceeds_max_minutes() {
        let mut r = valid_create();
        r.minutes = 1441;
        assert!(matches!(
            validate_create(&r),
            Err(EntryError::Validation(_))
        ));
    }

    #[test]
    fn create_zero_minutes_ok() {
        let mut r = valid_create();
        r.minutes = 0;
        assert!(validate_create(&r).is_ok());
    }

    #[test]
    fn create_max_minutes_ok() {
        let mut r = valid_create();
        r.minutes = 1440;
        assert!(validate_create(&r).is_ok());
    }

    fn valid_correct() -> CorrectEntryRequest {
        CorrectEntryRequest {
            app_id: "acme".to_string(),
            entry_id: Uuid::new_v4(),
            minutes: 240,
            description: Some("Corrected time".into()),
            project_id: None,
            task_id: None,
            created_by: None,
        }
    }

    #[test]
    fn correct_valid() {
        assert!(validate_correct(&valid_correct()).is_ok());
    }

    #[test]
    fn correct_negative_minutes() {
        let mut r = valid_correct();
        r.minutes = -10;
        assert!(matches!(
            validate_correct(&r),
            Err(EntryError::Validation(_))
        ));
    }

    #[test]
    fn void_valid() {
        let req = VoidEntryRequest {
            app_id: "acme".into(),
            entry_id: Uuid::new_v4(),
            created_by: None,
        };
        assert!(validate_void(&req).is_ok());
    }

    #[test]
    fn void_empty_app_id() {
        let req = VoidEntryRequest {
            app_id: "".into(),
            entry_id: Uuid::new_v4(),
            created_by: None,
        };
        assert!(matches!(
            validate_void(&req),
            Err(EntryError::Validation(_))
        ));
    }
}
