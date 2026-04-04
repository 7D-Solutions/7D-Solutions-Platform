//! Allocation service — CRUD for planned allocations + actual-time rollup queries.
//!
//! Allocations are simple CRUD (no Guard→Mutation→Outbox; they're planning data).
//! Rollups aggregate actual minutes from tk_timesheet_entries using deterministic
//! integer arithmetic (SUM of minutes column, no floating-point).

use chrono::NaiveDate;
use sqlx::PgPool;
use uuid::Uuid;

use super::models::*;
use super::repo;

// ============================================================================
// Allocation CRUD
// ============================================================================

pub async fn create_allocation(
    pool: &PgPool,
    req: &CreateAllocationRequest,
) -> Result<Allocation, AllocationError> {
    req.validate()?;
    repo::insert_allocation(pool, req).await
}

pub async fn update_allocation(
    pool: &PgPool,
    id: Uuid,
    req: &UpdateAllocationRequest,
) -> Result<Allocation, AllocationError> {
    req.validate()?;
    repo::update_allocation(pool, id, req)
        .await?
        .ok_or(AllocationError::NotFound)
}

pub async fn deactivate_allocation(
    pool: &PgPool,
    id: Uuid,
    app_id: &str,
) -> Result<Allocation, AllocationError> {
    repo::deactivate_allocation(pool, id, app_id)
        .await?
        .ok_or(AllocationError::NotFound)
}

pub async fn get_allocation(
    pool: &PgPool,
    id: Uuid,
    app_id: &str,
) -> Result<Allocation, AllocationError> {
    repo::get_allocation(pool, id, app_id)
        .await?
        .ok_or(AllocationError::NotFound)
}

pub async fn list_allocations(
    pool: &PgPool,
    app_id: &str,
    employee_id: Option<Uuid>,
    project_id: Option<Uuid>,
    active_only: bool,
) -> Result<Vec<Allocation>, AllocationError> {
    repo::list_allocations(pool, app_id, employee_id, project_id, active_only).await
}

// ============================================================================
// Rollup queries (actual time from tk_timesheet_entries)
// ============================================================================

/// Rollup actual minutes by project within a date range.
/// Only counts is_current = TRUE, entry_type != 'void'.
pub async fn rollup_by_project(
    pool: &PgPool,
    app_id: &str,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<ProjectRollup>, AllocationError> {
    repo::rollup_by_project(pool, app_id, from, to).await
}

/// Rollup actual minutes by employee within a date range.
pub async fn rollup_by_employee(
    pool: &PgPool,
    app_id: &str,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<EmployeeRollup>, AllocationError> {
    repo::rollup_by_employee(pool, app_id, from, to).await
}

/// Rollup actual minutes by task within a project and date range.
pub async fn rollup_by_task(
    pool: &PgPool,
    app_id: &str,
    project_id: Uuid,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<TaskRollup>, AllocationError> {
    repo::rollup_by_task(pool, app_id, project_id, from, to).await
}
