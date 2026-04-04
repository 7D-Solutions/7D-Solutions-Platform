//! Allocation repository — SQL layer for tk_allocations and rollup queries.

use chrono::NaiveDate;
use sqlx::PgPool;
use uuid::Uuid;

use super::models::*;

// ============================================================================
// Allocation CRUD
// ============================================================================

pub async fn insert_allocation(
    pool: &PgPool,
    req: &CreateAllocationRequest,
) -> Result<Allocation, AllocationError> {
    sqlx::query_as::<_, Allocation>(
        r#"
        INSERT INTO tk_allocations
            (app_id, employee_id, project_id, task_id,
             allocated_minutes_per_week, effective_from, effective_to)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        RETURNING *
        "#,
    )
    .bind(&req.app_id)
    .bind(req.employee_id)
    .bind(req.project_id)
    .bind(req.task_id)
    .bind(req.allocated_minutes_per_week)
    .bind(req.effective_from)
    .bind(req.effective_to)
    .fetch_one(pool)
    .await
    .map_err(AllocationError::Database)
}

pub async fn update_allocation(
    pool: &PgPool,
    id: Uuid,
    req: &UpdateAllocationRequest,
) -> Result<Option<Allocation>, AllocationError> {
    Ok(sqlx::query_as::<_, Allocation>(
        r#"
        UPDATE tk_allocations
        SET
            allocated_minutes_per_week = COALESCE($3, allocated_minutes_per_week),
            effective_to = CASE WHEN $4::DATE IS NOT NULL THEN $4 ELSE effective_to END,
            updated_at = NOW()
        WHERE id = $1 AND app_id = $2 AND active = TRUE
        RETURNING *
        "#,
    )
    .bind(id)
    .bind(&req.app_id)
    .bind(req.allocated_minutes_per_week)
    .bind(req.effective_to)
    .fetch_optional(pool)
    .await?)
}

pub async fn deactivate_allocation(
    pool: &PgPool,
    id: Uuid,
    app_id: &str,
) -> Result<Option<Allocation>, AllocationError> {
    Ok(sqlx::query_as::<_, Allocation>(
        r#"
        UPDATE tk_allocations
        SET active = FALSE, updated_at = NOW()
        WHERE id = $1 AND app_id = $2
        RETURNING *
        "#,
    )
    .bind(id)
    .bind(app_id)
    .fetch_optional(pool)
    .await?)
}

pub async fn get_allocation(
    pool: &PgPool,
    id: Uuid,
    app_id: &str,
) -> Result<Option<Allocation>, AllocationError> {
    Ok(
        sqlx::query_as::<_, Allocation>(
            "SELECT * FROM tk_allocations WHERE id = $1 AND app_id = $2",
        )
        .bind(id)
        .bind(app_id)
        .fetch_optional(pool)
        .await?,
    )
}

pub async fn list_allocations(
    pool: &PgPool,
    app_id: &str,
    employee_id: Option<Uuid>,
    project_id: Option<Uuid>,
    active_only: bool,
) -> Result<Vec<Allocation>, AllocationError> {
    Ok(sqlx::query_as::<_, Allocation>(
        r#"
        SELECT * FROM tk_allocations
        WHERE app_id = $1
          AND ($2::UUID IS NULL OR employee_id = $2)
          AND ($3::UUID IS NULL OR project_id = $3)
          AND ($4::BOOLEAN = FALSE OR active = TRUE)
        ORDER BY effective_from DESC
        "#,
    )
    .bind(app_id)
    .bind(employee_id)
    .bind(project_id)
    .bind(active_only)
    .fetch_all(pool)
    .await?)
}

// ============================================================================
// Rollup queries (actual time from tk_timesheet_entries)
// ============================================================================

pub async fn rollup_by_project(
    pool: &PgPool,
    app_id: &str,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<ProjectRollup>, AllocationError> {
    Ok(sqlx::query_as::<_, ProjectRollup>(
        r#"
        SELECT
            e.project_id,
            COALESCE(p.name, '(no project)') AS project_name,
            COALESCE(SUM(e.minutes), 0)::BIGINT AS total_minutes,
            COUNT(*)::BIGINT AS entry_count
        FROM tk_timesheet_entries e
        LEFT JOIN tk_projects p ON p.id = e.project_id
        WHERE e.app_id = $1
          AND e.work_date >= $2 AND e.work_date <= $3
          AND e.is_current = TRUE
          AND e.entry_type != 'void'
          AND e.project_id IS NOT NULL
        GROUP BY e.project_id, p.name
        ORDER BY total_minutes DESC
        "#,
    )
    .bind(app_id)
    .bind(from)
    .bind(to)
    .fetch_all(pool)
    .await?)
}

pub async fn rollup_by_employee(
    pool: &PgPool,
    app_id: &str,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<EmployeeRollup>, AllocationError> {
    Ok(sqlx::query_as::<_, EmployeeRollup>(
        r#"
        SELECT
            e.employee_id,
            emp.first_name,
            emp.last_name,
            COALESCE(SUM(e.minutes), 0)::BIGINT AS total_minutes,
            COUNT(*)::BIGINT AS entry_count
        FROM tk_timesheet_entries e
        JOIN tk_employees emp ON emp.id = e.employee_id
        WHERE e.app_id = $1
          AND e.work_date >= $2 AND e.work_date <= $3
          AND e.is_current = TRUE
          AND e.entry_type != 'void'
        GROUP BY e.employee_id, emp.first_name, emp.last_name
        ORDER BY total_minutes DESC
        "#,
    )
    .bind(app_id)
    .bind(from)
    .bind(to)
    .fetch_all(pool)
    .await?)
}

pub async fn rollup_by_task(
    pool: &PgPool,
    app_id: &str,
    project_id: Uuid,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<TaskRollup>, AllocationError> {
    Ok(sqlx::query_as::<_, TaskRollup>(
        r#"
        SELECT
            e.project_id,
            e.task_id,
            COALESCE(t.name, '(no task)') AS task_name,
            COALESCE(SUM(e.minutes), 0)::BIGINT AS total_minutes,
            COUNT(*)::BIGINT AS entry_count
        FROM tk_timesheet_entries e
        LEFT JOIN tk_tasks t ON t.id = e.task_id
        WHERE e.app_id = $1
          AND e.project_id = $2
          AND e.work_date >= $3 AND e.work_date <= $4
          AND e.is_current = TRUE
          AND e.entry_type != 'void'
          AND e.task_id IS NOT NULL
        GROUP BY e.project_id, e.task_id, t.name
        ORDER BY total_minutes DESC
        "#,
    )
    .bind(app_id)
    .bind(project_id)
    .bind(from)
    .bind(to)
    .fetch_all(pool)
    .await?)
}
