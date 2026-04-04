//! Timesheet entry repository — SQL layer for tk_timesheet_entries.

use chrono::NaiveDate;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::models::{EntryError, TimesheetEntry};

// ============================================================================
// Reads (pool-based)
// ============================================================================

pub async fn list_entries(
    pool: &PgPool,
    app_id: &str,
    employee_id: Uuid,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<TimesheetEntry>, EntryError> {
    Ok(sqlx::query_as::<_, TimesheetEntry>(
        r#"
        SELECT id, entry_id, version, app_id, employee_id, project_id, task_id,
               work_date, minutes, description, entry_type, is_current,
               created_by, created_at
        FROM tk_timesheet_entries
        WHERE app_id = $1 AND employee_id = $2
          AND work_date >= $3 AND work_date <= $4
          AND is_current = TRUE
        ORDER BY work_date, created_at
        "#,
    )
    .bind(app_id)
    .bind(employee_id)
    .bind(from)
    .bind(to)
    .fetch_all(pool)
    .await?)
}

pub async fn entry_history(
    pool: &PgPool,
    app_id: &str,
    entry_id: Uuid,
) -> Result<Vec<TimesheetEntry>, EntryError> {
    Ok(sqlx::query_as::<_, TimesheetEntry>(
        r#"
        SELECT id, entry_id, version, app_id, employee_id, project_id, task_id,
               work_date, minutes, description, entry_type, is_current,
               created_by, created_at
        FROM tk_timesheet_entries
        WHERE app_id = $1 AND entry_id = $2
        ORDER BY version ASC
        "#,
    )
    .bind(app_id)
    .bind(entry_id)
    .fetch_all(pool)
    .await?)
}

// ============================================================================
// Guard queries (pool-based)
// ============================================================================

pub async fn is_period_locked(
    pool: &PgPool,
    app_id: &str,
    employee_id: Uuid,
    work_date: NaiveDate,
) -> Result<bool, EntryError> {
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
    Ok(locked.is_some())
}

pub async fn has_overlap(
    pool: &PgPool,
    app_id: &str,
    employee_id: Uuid,
    work_date: NaiveDate,
    project_id: Option<Uuid>,
    task_id: Option<Uuid>,
) -> Result<bool, EntryError> {
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
    Ok(exists.is_some())
}

// ============================================================================
// Writes (conn-based, called within transactions)
// ============================================================================

pub async fn insert_entry(
    conn: &mut PgConnection,
    entry_id: Uuid,
    app_id: &str,
    employee_id: Uuid,
    project_id: Option<Uuid>,
    task_id: Option<Uuid>,
    work_date: NaiveDate,
    minutes: i32,
    description: Option<&str>,
    created_by: Option<Uuid>,
) -> Result<TimesheetEntry, EntryError> {
    Ok(sqlx::query_as::<_, TimesheetEntry>(
        r#"
        INSERT INTO tk_timesheet_entries
            (entry_id, version, app_id, employee_id, project_id, task_id,
             work_date, minutes, description, entry_type, is_current, created_by)
        VALUES ($1, 1, $2, $3, $4, $5, $6, $7, $8, 'original', TRUE, $9)
        RETURNING id, entry_id, version, app_id, employee_id, project_id, task_id,
                  work_date, minutes, description, entry_type, is_current,
                  created_by, created_at
        "#,
    )
    .bind(entry_id)
    .bind(app_id)
    .bind(employee_id)
    .bind(project_id)
    .bind(task_id)
    .bind(work_date)
    .bind(minutes)
    .bind(description)
    .bind(created_by)
    .fetch_one(conn)
    .await?)
}

pub async fn fetch_current_for_update(
    conn: &mut PgConnection,
    app_id: &str,
    entry_id: Uuid,
) -> Result<Option<TimesheetEntry>, EntryError> {
    Ok(sqlx::query_as::<_, TimesheetEntry>(
        r#"
        SELECT id, entry_id, version, app_id, employee_id, project_id, task_id,
               work_date, minutes, description, entry_type, is_current,
               created_by, created_at
        FROM tk_timesheet_entries
        WHERE app_id = $1 AND entry_id = $2 AND is_current = TRUE
        FOR UPDATE
        "#,
    )
    .bind(app_id)
    .bind(entry_id)
    .fetch_optional(conn)
    .await?)
}

pub async fn flip_is_current(
    conn: &mut PgConnection,
    entry_id: Uuid,
) -> Result<(), EntryError> {
    sqlx::query(
        "UPDATE tk_timesheet_entries SET is_current = FALSE \
         WHERE entry_id = $1 AND is_current = TRUE",
    )
    .bind(entry_id)
    .execute(conn)
    .await?;
    Ok(())
}

pub async fn insert_correction(
    conn: &mut PgConnection,
    entry_id: Uuid,
    version: i32,
    app_id: &str,
    employee_id: Uuid,
    project_id: Option<Uuid>,
    task_id: Option<Uuid>,
    work_date: NaiveDate,
    minutes: i32,
    description: Option<&str>,
    created_by: Option<Uuid>,
) -> Result<TimesheetEntry, EntryError> {
    Ok(sqlx::query_as::<_, TimesheetEntry>(
        r#"
        INSERT INTO tk_timesheet_entries
            (entry_id, version, app_id, employee_id, project_id, task_id,
             work_date, minutes, description, entry_type, is_current, created_by)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'correction', TRUE, $10)
        RETURNING id, entry_id, version, app_id, employee_id, project_id, task_id,
                  work_date, minutes, description, entry_type, is_current,
                  created_by, created_at
        "#,
    )
    .bind(entry_id)
    .bind(version)
    .bind(app_id)
    .bind(employee_id)
    .bind(project_id)
    .bind(task_id)
    .bind(work_date)
    .bind(minutes)
    .bind(description)
    .bind(created_by)
    .fetch_one(conn)
    .await?)
}

pub async fn insert_void(
    conn: &mut PgConnection,
    entry_id: Uuid,
    version: i32,
    app_id: &str,
    employee_id: Uuid,
    project_id: Option<Uuid>,
    task_id: Option<Uuid>,
    work_date: NaiveDate,
    created_by: Option<Uuid>,
) -> Result<TimesheetEntry, EntryError> {
    Ok(sqlx::query_as::<_, TimesheetEntry>(
        r#"
        INSERT INTO tk_timesheet_entries
            (entry_id, version, app_id, employee_id, project_id, task_id,
             work_date, minutes, description, entry_type, is_current, created_by)
        VALUES ($1, $2, $3, $4, $5, $6, $7, 0, 'Voided', 'void', TRUE, $8)
        RETURNING id, entry_id, version, app_id, employee_id, project_id, task_id,
                  work_date, minutes, description, entry_type, is_current,
                  created_by, created_at
        "#,
    )
    .bind(entry_id)
    .bind(version)
    .bind(app_id)
    .bind(employee_id)
    .bind(project_id)
    .bind(task_id)
    .bind(work_date)
    .bind(created_by)
    .fetch_one(conn)
    .await?)
}
