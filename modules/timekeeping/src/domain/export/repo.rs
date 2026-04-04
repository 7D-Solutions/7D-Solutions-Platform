//! Export repository — SQL layer for tk_export_runs.

use chrono::{DateTime, NaiveDate, Utc};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::models::*;

const EXPORT_COLS: &str = r#"
    id, app_id, export_type, period_start, period_end, status,
    record_count, metadata, content_hash, error_message,
    started_at, completed_at, created_at
"#;

// ============================================================================
// Reads
// ============================================================================

pub async fn fetch_export_run(
    pool: &PgPool,
    app_id: &str,
    run_id: Uuid,
) -> Result<Option<ExportRun>, ExportError> {
    let sql = format!(
        "SELECT {} FROM tk_export_runs WHERE app_id = $1 AND id = $2",
        EXPORT_COLS
    );
    Ok(sqlx::query_as::<_, ExportRun>(&sql)
        .bind(app_id)
        .bind(run_id)
        .fetch_optional(pool)
        .await?)
}

pub async fn list_export_runs(
    pool: &PgPool,
    app_id: &str,
    export_type: Option<&str>,
) -> Result<Vec<ExportRun>, ExportError> {
    if let Some(et) = export_type {
        let sql = format!(
            "SELECT {} FROM tk_export_runs \
             WHERE app_id = $1 AND export_type = $2 \
             ORDER BY created_at DESC",
            EXPORT_COLS
        );
        Ok(sqlx::query_as::<_, ExportRun>(&sql)
            .bind(app_id)
            .bind(et)
            .fetch_all(pool)
            .await?)
    } else {
        let sql = format!(
            "SELECT {} FROM tk_export_runs WHERE app_id = $1 ORDER BY created_at DESC",
            EXPORT_COLS
        );
        Ok(sqlx::query_as::<_, ExportRun>(&sql)
            .bind(app_id)
            .fetch_all(pool)
            .await?)
    }
}

pub async fn fetch_approved_entries(
    pool: &PgPool,
    app_id: &str,
    period_start: NaiveDate,
    period_end: NaiveDate,
) -> Result<Vec<ExportEntry>, ExportError> {
    Ok(sqlx::query_as::<_, ExportEntry>(
        r#"
        SELECT
            e.entry_id,
            e.employee_id,
            COALESCE(emp.first_name || ' ' || emp.last_name, 'Unknown') AS employee_name,
            e.project_id,
            p.name AS project_name,
            e.task_id,
            e.work_date,
            e.minutes,
            e.description
        FROM tk_timesheet_entries e
        JOIN tk_approval_requests ar
            ON ar.app_id = e.app_id
            AND ar.employee_id = e.employee_id
            AND ar.period_start <= e.work_date
            AND ar.period_end >= e.work_date
            AND ar.status = 'approved'
        LEFT JOIN tk_employees emp ON emp.id = e.employee_id
        LEFT JOIN tk_projects p ON p.id = e.project_id
        WHERE e.app_id = $1
          AND e.work_date >= $2
          AND e.work_date <= $3
          AND e.is_current = TRUE
          AND e.entry_type != 'void'
        ORDER BY e.work_date, e.employee_id, e.entry_id
        "#,
    )
    .bind(app_id)
    .bind(period_start)
    .bind(period_end)
    .fetch_all(pool)
    .await?)
}

pub async fn find_by_hash(
    pool: &PgPool,
    app_id: &str,
    export_type: &str,
    period_start: NaiveDate,
    period_end: NaiveDate,
    content_hash: &str,
) -> Result<Option<ExportRun>, ExportError> {
    let sql = format!(
        "SELECT {} FROM tk_export_runs \
         WHERE app_id = $1 AND export_type = $2 \
         AND period_start = $3 AND period_end = $4 \
         AND content_hash = $5 LIMIT 1",
        EXPORT_COLS
    );
    Ok(sqlx::query_as::<_, ExportRun>(&sql)
        .bind(app_id)
        .bind(export_type)
        .bind(period_start)
        .bind(period_end)
        .bind(content_hash)
        .fetch_optional(pool)
        .await?)
}

// ============================================================================
// Writes
// ============================================================================

pub async fn insert_export_run(
    conn: &mut PgConnection,
    run_id: Uuid,
    app_id: &str,
    export_type: &str,
    period_start: NaiveDate,
    period_end: NaiveDate,
    record_count: i32,
    json_content: &serde_json::Value,
    content_hash: &str,
    now: DateTime<Utc>,
) -> Result<ExportRun, ExportError> {
    Ok(sqlx::query_as::<_, ExportRun>(&format!(
        r#"INSERT INTO tk_export_runs
            (id, app_id, export_type, period_start, period_end, status,
             record_count, metadata, content_hash, started_at, completed_at)
        VALUES ($1, $2, $3, $4, $5, 'completed', $6, $7, $8, $9, $9)
        RETURNING {}"#,
        EXPORT_COLS
    ))
    .bind(run_id)
    .bind(app_id)
    .bind(export_type)
    .bind(period_start)
    .bind(period_end)
    .bind(record_count)
    .bind(json_content)
    .bind(content_hash)
    .bind(now)
    .fetch_one(conn)
    .await?)
}
