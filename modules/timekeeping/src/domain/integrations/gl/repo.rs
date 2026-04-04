//! GL integration repository — SQL queries for labor cost aggregation.

use chrono::NaiveDate;
use sqlx::PgPool;
use uuid::Uuid;

/// Row returned when querying approved time with employee rates.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct LaborCostRow {
    pub employee_id: Uuid,
    pub employee_name: String,
    pub project_id: Option<Uuid>,
    pub project_name: Option<String>,
    pub total_minutes: i64,
    pub hourly_rate_minor: i64,
    pub currency: String,
}

pub async fn fetch_labor_cost_rows(
    pool: &PgPool,
    app_id: &str,
    period_start: NaiveDate,
    period_end: NaiveDate,
) -> Result<Vec<LaborCostRow>, sqlx::Error> {
    sqlx::query_as::<_, LaborCostRow>(
        r#"
        SELECT
            e.employee_id,
            COALESCE(emp.first_name || ' ' || emp.last_name, 'Unknown') AS employee_name,
            e.project_id,
            p.name AS project_name,
            SUM(e.minutes)::BIGINT AS total_minutes,
            emp.hourly_rate_minor,
            emp.currency
        FROM tk_timesheet_entries e
        JOIN tk_approval_requests ar
            ON ar.app_id = e.app_id
            AND ar.employee_id = e.employee_id
            AND ar.period_start <= e.work_date
            AND ar.period_end >= e.work_date
            AND ar.status = 'approved'
        JOIN tk_employees emp
            ON emp.id = e.employee_id
            AND emp.hourly_rate_minor IS NOT NULL
        LEFT JOIN tk_projects p ON p.id = e.project_id
        WHERE e.app_id = $1
          AND e.work_date >= $2
          AND e.work_date <= $3
          AND e.is_current = TRUE
          AND e.entry_type != 'void'
        GROUP BY e.employee_id, emp.first_name, emp.last_name,
                 e.project_id, p.name,
                 emp.hourly_rate_minor, emp.currency
        ORDER BY e.employee_id, e.project_id
        "#,
    )
    .bind(app_id)
    .bind(period_start)
    .bind(period_end)
    .fetch_all(pool)
    .await
}
