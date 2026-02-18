//! GL labor cost accrual integration — generates GL posting requests
//! from approved timesheet entries.
//!
//! ## Accounting Entry
//!
//! ```text
//! DR  LABOR_EXPENSE   total_cost  ← labor cost recognized
//! CR  ACCRUED_LABOR   total_cost  ← accrued labor liability
//! ```
//!
//! Cost is computed as `(minutes / 60.0) * hourly_rate_minor / 100.0`
//! using the employee's configured `hourly_rate_minor`.
//!
//! ## Exactly-once Semantics
//!
//! Each posting carries a deterministic `posting_id` derived from
//! (app_id, employee_id, period_start, period_end) via UUID v5. Re-running
//! the same period produces the same posting_id, which the GL consumer's
//! `processed_events` table uses as the idempotency key.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

use crate::events;

// ============================================================================
// GL labor cost posting payload (published via outbox)
// ============================================================================

/// Payload emitted to the outbox for GL consumption.
///
/// The GL consumer deserializes this to build a `GlPostingRequestV1`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaborCostPostingPayload {
    pub posting_id: Uuid,
    pub app_id: String,
    pub employee_id: Uuid,
    pub employee_name: String,
    pub project_id: Option<Uuid>,
    pub project_name: Option<String>,
    pub period_start: NaiveDate,
    pub period_end: NaiveDate,
    pub total_minutes: i32,
    pub hourly_rate_minor: i64,
    pub currency: String,
    pub total_cost_minor: i64,
    pub posting_date: String,
}

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

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug, Error)]
pub enum GlIntegrationError {
    #[error("No approved entries with configured rates for this period")]
    NoEligibleEntries,

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ============================================================================
// Service
// ============================================================================

const EVT_LABOR_COST_POSTING: &str = "timekeeping.labor_cost";

/// UUID namespace for deterministic posting IDs.
const LABOR_COST_NS: Uuid = Uuid::from_bytes([
    0x7d, 0x50, 0x1a, 0xb0, 0xcc, 0x01, 0x4e, 0x2f, 0x8a, 0x11, 0x3c, 0xd4, 0xe5, 0xf6, 0xa7,
    0xb8,
]);

/// Generate GL labor cost postings for approved time in a period.
///
/// Queries approved entries joined with employee rates, groups by
/// (employee, project), and enqueues a posting event per group into
/// the outbox. Returns the list of posting payloads emitted.
pub async fn generate_labor_cost_postings(
    pool: &PgPool,
    app_id: &str,
    period_start: NaiveDate,
    period_end: NaiveDate,
) -> Result<Vec<LaborCostPostingPayload>, GlIntegrationError> {
    if app_id.trim().is_empty() {
        return Err(GlIntegrationError::Validation(
            "app_id must not be empty".into(),
        ));
    }
    if period_end < period_start {
        return Err(GlIntegrationError::Validation(
            "period_end must be >= period_start".into(),
        ));
    }

    // Fetch approved entries grouped by employee + project, joined with rates
    let rows = fetch_labor_cost_rows(pool, app_id, period_start, period_end).await?;
    if rows.is_empty() {
        return Err(GlIntegrationError::NoEligibleEntries);
    }

    let posting_date = period_end.format("%Y-%m-%d").to_string();
    let mut payloads = Vec::with_capacity(rows.len());
    let mut tx = pool.begin().await?;

    for row in &rows {
        // Cost = (minutes / 60) * hourly_rate_minor (in minor units)
        let total_cost_minor = (row.total_minutes * row.hourly_rate_minor) / 60;

        // Deterministic posting ID: hash of (app_id, employee_id, project_id, period)
        let id_seed = format!(
            "{}:{}:{}:{}:{}",
            app_id,
            row.employee_id,
            row.project_id
                .map(|p| p.to_string())
                .unwrap_or_default(),
            period_start,
            period_end,
        );
        let posting_id = Uuid::new_v5(&LABOR_COST_NS, id_seed.as_bytes());

        let payload = LaborCostPostingPayload {
            posting_id,
            app_id: app_id.to_string(),
            employee_id: row.employee_id,
            employee_name: row.employee_name.clone(),
            project_id: row.project_id,
            project_name: row.project_name.clone(),
            period_start,
            period_end,
            total_minutes: row.total_minutes as i32,
            hourly_rate_minor: row.hourly_rate_minor,
            currency: row.currency.clone(),
            total_cost_minor,
            posting_date: posting_date.clone(),
        };

        events::enqueue_event_tx(
            &mut tx,
            posting_id,
            EVT_LABOR_COST_POSTING,
            "labor_cost",
            &posting_id.to_string(),
            &payload,
        )
        .await?;

        payloads.push(payload);
    }

    tx.commit().await?;
    Ok(payloads)
}

// ============================================================================
// Internal helpers
// ============================================================================

/// Fetch approved time grouped by (employee, project) with hourly rates.
///
/// Only includes employees with a configured `hourly_rate_minor`.
async fn fetch_labor_cost_rows(
    pool: &PgPool,
    app_id: &str,
    period_start: NaiveDate,
    period_end: NaiveDate,
) -> Result<Vec<LaborCostRow>, GlIntegrationError> {
    let rows = sqlx::query_as::<_, LaborCostRow>(
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
    fn posting_id_is_deterministic() {
        let seed = "acme:00000000-0000-0000-0000-000000000001::2026-02-01:2026-02-07";
        let id1 = Uuid::new_v5(&LABOR_COST_NS, seed.as_bytes());
        let id2 = Uuid::new_v5(&LABOR_COST_NS, seed.as_bytes());
        assert_eq!(id1, id2);
    }

    #[test]
    fn posting_id_differs_by_employee() {
        let seed_a = "acme:emp-a::2026-02-01:2026-02-07";
        let seed_b = "acme:emp-b::2026-02-01:2026-02-07";
        let id_a = Uuid::new_v5(&LABOR_COST_NS, seed_a.as_bytes());
        let id_b = Uuid::new_v5(&LABOR_COST_NS, seed_b.as_bytes());
        assert_ne!(id_a, id_b);
    }

    #[test]
    fn payload_serde_roundtrip() {
        let payload = LaborCostPostingPayload {
            posting_id: Uuid::new_v4(),
            app_id: "acme".into(),
            employee_id: Uuid::new_v4(),
            employee_name: "Jane Doe".into(),
            project_id: None,
            project_name: None,
            period_start: NaiveDate::from_ymd_opt(2026, 2, 1).unwrap(),
            period_end: NaiveDate::from_ymd_opt(2026, 2, 7).unwrap(),
            total_minutes: 2400,
            hourly_rate_minor: 5000,
            currency: "USD".into(),
            total_cost_minor: 200000,
            posting_date: "2026-02-07".into(),
        };
        let json = serde_json::to_string(&payload).unwrap();
        let back: LaborCostPostingPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(back.app_id, "acme");
        assert_eq!(back.total_cost_minor, 200000);
    }
}
