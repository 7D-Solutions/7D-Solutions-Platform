//! AR billable time integration — exports approved billable time
//! as draft invoice line items for AR consumption.
//!
//! Only entries on projects with `billable = true` are exported.
//! Each export carries a deterministic `export_id` derived from
//! (app_id, employee_id, project_id, period) so AR can deduplicate.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

use crate::events;

// ============================================================================
// Billable time line item (published via outbox for AR consumption)
// ============================================================================

/// A single billable time line item for AR invoice generation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BillableTimeLine {
    pub employee_id: Uuid,
    pub employee_name: String,
    pub project_id: Uuid,
    pub project_name: String,
    pub total_minutes: i32,
    pub hourly_rate_minor: i64,
    pub currency: String,
    pub amount_minor: i64,
    pub description: String,
}

/// Payload emitted to the outbox for AR consumption.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BillableTimeExportPayload {
    pub export_id: Uuid,
    pub app_id: String,
    pub period_start: NaiveDate,
    pub period_end: NaiveDate,
    pub lines: Vec<BillableTimeLine>,
    pub total_amount_minor: i64,
    pub currency: String,
}

/// Row returned when querying billable approved time.
#[derive(Debug, Clone, sqlx::FromRow)]
struct BillableTimeRow {
    pub employee_id: Uuid,
    pub employee_name: String,
    pub project_id: Uuid,
    pub project_name: String,
    pub total_minutes: i64,
    pub hourly_rate_minor: i64,
    pub currency: String,
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug, Error)]
pub enum ArIntegrationError {
    #[error("No billable approved entries for this period")]
    NoBillableEntries,

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ============================================================================
// Service
// ============================================================================

const EVT_BILLABLE_TIME_EXPORT: &str = "timekeeping.billable_time";

/// UUID namespace for deterministic export IDs.
const BILLABLE_TIME_NS: Uuid = Uuid::from_bytes([
    0x7d, 0x50, 0x2b, 0xc1, 0xdd, 0x02, 0x4f, 0x3a, 0x9b, 0x22, 0x4d, 0xe5, 0xf6, 0x07, 0xb8, 0xc9,
]);

/// Export billable time for a period as AR draft invoice lines.
///
/// Queries approved entries on billable projects, groups by
/// (employee, project), and enqueues a single export event into
/// the outbox with all line items.
pub async fn export_billable_time(
    pool: &PgPool,
    app_id: &str,
    period_start: NaiveDate,
    period_end: NaiveDate,
) -> Result<BillableTimeExportPayload, ArIntegrationError> {
    if app_id.trim().is_empty() {
        return Err(ArIntegrationError::Validation(
            "app_id must not be empty".into(),
        ));
    }
    if period_end < period_start {
        return Err(ArIntegrationError::Validation(
            "period_end must be >= period_start".into(),
        ));
    }

    let rows = fetch_billable_rows(pool, app_id, period_start, period_end).await?;
    if rows.is_empty() {
        return Err(ArIntegrationError::NoBillableEntries);
    }

    // Deterministic export ID
    let id_seed = format!("{}:{}:{}", app_id, period_start, period_end);
    let export_id = Uuid::new_v5(&BILLABLE_TIME_NS, id_seed.as_bytes());

    let currency = rows[0].currency.clone();

    let lines: Vec<BillableTimeLine> = rows
        .iter()
        .map(|r| {
            let amount_minor = (r.total_minutes * r.hourly_rate_minor) / 60;
            let hours = r.total_minutes as f64 / 60.0;
            BillableTimeLine {
                employee_id: r.employee_id,
                employee_name: r.employee_name.clone(),
                project_id: r.project_id,
                project_name: r.project_name.clone(),
                total_minutes: r.total_minutes as i32,
                hourly_rate_minor: r.hourly_rate_minor,
                currency: r.currency.clone(),
                amount_minor,
                description: format!("{:.1}h — {} on {}", hours, r.employee_name, r.project_name),
            }
        })
        .collect();

    let total_amount_minor: i64 = lines.iter().map(|l| l.amount_minor).sum();

    let payload = BillableTimeExportPayload {
        export_id,
        app_id: app_id.to_string(),
        period_start,
        period_end,
        lines,
        total_amount_minor,
        currency,
    };

    let mut tx = pool.begin().await?;
    events::enqueue_event_tx(
        &mut tx,
        export_id,
        EVT_BILLABLE_TIME_EXPORT,
        "billable_time_export",
        &export_id.to_string(),
        &payload,
    )
    .await?;
    tx.commit().await?;

    Ok(payload)
}

// ============================================================================
// Internal helpers
// ============================================================================

async fn fetch_billable_rows(
    pool: &PgPool,
    app_id: &str,
    period_start: NaiveDate,
    period_end: NaiveDate,
) -> Result<Vec<BillableTimeRow>, ArIntegrationError> {
    let rows = sqlx::query_as::<_, BillableTimeRow>(
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
        JOIN tk_projects p
            ON p.id = e.project_id
            AND p.billable = TRUE
        WHERE e.app_id = $1
          AND e.work_date >= $2
          AND e.work_date <= $3
          AND e.is_current = TRUE
          AND e.entry_type != 'void'
          AND e.project_id IS NOT NULL
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
    .await?;

    Ok(rows)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_id_is_deterministic() {
        let seed = "acme:2026-02-01:2026-02-07";
        let id1 = Uuid::new_v5(&BILLABLE_TIME_NS, seed.as_bytes());
        let id2 = Uuid::new_v5(&BILLABLE_TIME_NS, seed.as_bytes());
        assert_eq!(id1, id2);
    }

    #[test]
    fn billable_line_amount_calculation() {
        let line = BillableTimeLine {
            employee_id: Uuid::new_v4(),
            employee_name: "Jane Doe".into(),
            project_id: Uuid::new_v4(),
            project_name: "Widget".into(),
            total_minutes: 120,
            hourly_rate_minor: 5000,
            currency: "USD".into(),
            amount_minor: (120 * 5000) / 60,
            description: "2.0h — Jane Doe on Widget".into(),
        };
        assert_eq!(line.amount_minor, 10000); // $100.00
    }

    #[test]
    fn payload_serde_roundtrip() {
        let payload = BillableTimeExportPayload {
            export_id: Uuid::new_v4(),
            app_id: "acme".into(),
            period_start: NaiveDate::from_ymd_opt(2026, 2, 1).unwrap(),
            period_end: NaiveDate::from_ymd_opt(2026, 2, 7).unwrap(),
            lines: vec![],
            total_amount_minor: 0,
            currency: "USD".into(),
        };
        let json = serde_json::to_string(&payload).unwrap();
        let back: BillableTimeExportPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(back.app_id, "acme");
    }
}
