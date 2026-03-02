//! Maintenance plans and plan assignments domain model.
//!
//! Invariants:
//! - plan name is unique per tenant (not DB-enforced, application guard)
//! - schedule_type determines which interval fields are required:
//!   - calendar: calendar_interval_days required
//!   - meter: meter_type_id + meter_interval required
//!   - both: all three required
//! - next_due fields are deterministically computed by `recompute_due`
//! - Every query filters by tenant_id for multi-tenant isolation

mod repo;

pub use repo::{AssignmentRepo, PlanRepo};

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use super::work_orders::types::{Priority, ScheduleType};

// ============================================================================
// Domain models
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct MaintenancePlan {
    pub id: Uuid,
    pub tenant_id: String,
    pub name: String,
    pub description: Option<String>,
    pub asset_type_filter: Option<String>,
    #[sqlx(try_from = "String")]
    pub schedule_type: ScheduleType,
    pub calendar_interval_days: Option<i32>,
    pub meter_type_id: Option<Uuid>,
    pub meter_interval: Option<i64>,
    #[sqlx(try_from = "String")]
    pub priority: Priority,
    pub estimated_duration_minutes: Option<i32>,
    pub estimated_cost_minor: Option<i64>,
    pub task_checklist: Option<serde_json::Value>,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct PlanAssignment {
    pub id: Uuid,
    pub tenant_id: String,
    pub plan_id: Uuid,
    pub asset_id: Uuid,
    pub last_completed_at: Option<DateTime<Utc>>,
    pub last_meter_reading: Option<i64>,
    pub next_due_date: Option<NaiveDate>,
    pub next_due_meter: Option<i64>,
    pub state: String,
    pub due_notified_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// TryFrom<String> impls for sqlx
impl TryFrom<String> for ScheduleType {
    type Error = String;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::from_str_value(&s).map_err(|e| e.to_string())
    }
}

impl TryFrom<String> for Priority {
    type Error = String;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::from_str_value(&s).map_err(|e| e.to_string())
    }
}

// ============================================================================
// Request types
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct CreatePlanRequest {
    pub tenant_id: String,
    pub name: String,
    pub description: Option<String>,
    pub asset_type_filter: Option<String>,
    pub schedule_type: String,
    pub calendar_interval_days: Option<i32>,
    pub meter_type_id: Option<Uuid>,
    pub meter_interval: Option<i64>,
    pub priority: Option<String>,
    pub estimated_duration_minutes: Option<i32>,
    pub estimated_cost_minor: Option<i64>,
    pub task_checklist: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct UpdatePlanRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub priority: Option<String>,
    pub estimated_duration_minutes: Option<i32>,
    pub estimated_cost_minor: Option<i64>,
    pub task_checklist: Option<serde_json::Value>,
    pub is_active: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct ListPlansQuery {
    pub tenant_id: String,
    pub is_active: Option<bool>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct AssignPlanRequest {
    pub tenant_id: String,
    pub asset_id: Uuid,
}

#[derive(Debug, Deserialize)]
pub struct ListAssignmentsQuery {
    pub tenant_id: String,
    pub plan_id: Option<Uuid>,
    pub asset_id: Option<Uuid>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug, Error)]
pub enum PlanError {
    #[error("Plan not found")]
    PlanNotFound,

    #[error("Asset not found")]
    AssetNotFound,

    #[error("Assignment already exists for this plan and asset")]
    DuplicateAssignment,

    #[error("Assignment not found")]
    AssignmentNotFound,

    #[error("Meter type not found")]
    MeterTypeNotFound,

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ============================================================================
// recompute_due — pure function
// ============================================================================

/// Deterministically compute next_due_date and next_due_meter.
///
/// Rules:
/// - calendar: next_due_date = base_date + interval_days
///   (base_date = last_completed_at or now)
/// - meter: next_due_meter = base_meter + meter_interval
///   (base_meter = last_meter_reading or latest_reading or 0)
/// - both: compute both fields
pub fn recompute_due(
    schedule_type: ScheduleType,
    calendar_interval_days: Option<i32>,
    meter_interval: Option<i64>,
    last_completed_at: Option<DateTime<Utc>>,
    last_meter_reading: Option<i64>,
    latest_reading: Option<i64>,
    now: DateTime<Utc>,
) -> (Option<NaiveDate>, Option<i64>) {
    let compute_date = || -> Option<NaiveDate> {
        let days = calendar_interval_days?;
        let base = last_completed_at.unwrap_or(now);
        Some(base.date_naive() + chrono::Duration::days(days as i64))
    };

    let compute_meter = || -> Option<i64> {
        let interval = meter_interval?;
        let base = last_meter_reading.or(latest_reading).unwrap_or(0);
        Some(base + interval)
    };

    match schedule_type {
        ScheduleType::Calendar => (compute_date(), None),
        ScheduleType::Meter => (None, compute_meter()),
        ScheduleType::Both => (compute_date(), compute_meter()),
    }
}

// ============================================================================
// Unit tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn calendar_recompute_from_now() {
        let now = Utc.with_ymd_and_hms(2026, 3, 1, 12, 0, 0).unwrap();
        let (date, meter) = recompute_due(
            ScheduleType::Calendar,
            Some(30),
            None,
            None,
            None,
            None,
            now,
        );
        assert_eq!(date, Some(NaiveDate::from_ymd_opt(2026, 3, 31).unwrap()));
        assert_eq!(meter, None);
    }

    #[test]
    fn calendar_recompute_from_last_completed() {
        let now = Utc.with_ymd_and_hms(2026, 3, 15, 12, 0, 0).unwrap();
        let completed = Utc.with_ymd_and_hms(2026, 3, 1, 0, 0, 0).unwrap();
        let (date, meter) = recompute_due(
            ScheduleType::Calendar,
            Some(30),
            None,
            Some(completed),
            None,
            None,
            now,
        );
        assert_eq!(date, Some(NaiveDate::from_ymd_opt(2026, 3, 31).unwrap()));
        assert_eq!(meter, None);
    }

    #[test]
    fn meter_recompute_from_latest_reading() {
        let now = Utc::now();
        let (date, meter) = recompute_due(
            ScheduleType::Meter,
            None,
            Some(5000),
            None,
            None,
            Some(10_000),
            now,
        );
        assert_eq!(date, None);
        assert_eq!(meter, Some(15_000));
    }

    #[test]
    fn meter_recompute_from_last_meter_reading() {
        let now = Utc::now();
        let (date, meter) = recompute_due(
            ScheduleType::Meter,
            None,
            Some(5000),
            None,
            Some(20_000),
            Some(10_000),
            now,
        );
        assert_eq!(date, None);
        assert_eq!(meter, Some(25_000));
    }

    #[test]
    fn meter_recompute_no_readings() {
        let now = Utc::now();
        let (date, meter) =
            recompute_due(ScheduleType::Meter, None, Some(5000), None, None, None, now);
        assert_eq!(date, None);
        assert_eq!(meter, Some(5000));
    }

    #[test]
    fn both_recompute() {
        let now = Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap();
        let (date, meter) = recompute_due(
            ScheduleType::Both,
            Some(90),
            Some(5000),
            None,
            None,
            Some(50_000),
            now,
        );
        assert_eq!(date, Some(NaiveDate::from_ymd_opt(2026, 8, 30).unwrap()));
        assert_eq!(meter, Some(55_000));
    }
}
