//! Timekeeping → AR billable time integration.
//!
//! Receives `timekeeping.billable_time` events and records them as
//! pending billable line items. These can then be included in the
//! next AR invoice run for the relevant customer/project.
//!
//! ## Idempotency
//! The `export_id` from the timekeeping payload is stored in an
//! `ar_tk_billable_imports` table. Duplicate export_ids are skipped.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

// ============================================================================
// Inbound payload (mirrors timekeeping::integrations::ar::service)
// ============================================================================

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

// ============================================================================
// Import result
// ============================================================================

#[derive(Debug, Clone, Serialize)]
pub struct BillableTimeImportResult {
    pub export_id: Uuid,
    pub lines_imported: usize,
    pub total_amount_minor: i64,
    pub already_imported: bool,
}

// ============================================================================
// Service
// ============================================================================

/// Import billable time from timekeeping into AR.
///
/// Checks for duplicate `export_id` first. If already imported, returns
/// `already_imported = true` (idempotent). Otherwise records each line
/// as a pending billable item in `ar_tk_billable_imports`.
///
/// The table is created lazily if it doesn't exist (safe for tests).
pub async fn import_billable_time(
    pool: &PgPool,
    payload: &BillableTimeExportPayload,
) -> Result<BillableTimeImportResult, sqlx::Error> {
    // Ensure the import tracking table exists
    ensure_import_table(pool).await?;

    // Idempotency check
    let existing: Option<(Uuid,)> =
        sqlx::query_as("SELECT export_id FROM ar_tk_billable_imports WHERE export_id = $1 LIMIT 1")
            .bind(payload.export_id)
            .fetch_optional(pool)
            .await?;

    if existing.is_some() {
        return Ok(BillableTimeImportResult {
            export_id: payload.export_id,
            lines_imported: 0,
            total_amount_minor: payload.total_amount_minor,
            already_imported: true,
        });
    }

    let mut tx = pool.begin().await?;

    for line in &payload.lines {
        sqlx::query(
            r#"
            INSERT INTO ar_tk_billable_imports
                (export_id, app_id, employee_id, employee_name,
                 project_id, project_name, period_start, period_end,
                 total_minutes, hourly_rate_minor, amount_minor,
                 currency, description)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
            "#,
        )
        .bind(payload.export_id)
        .bind(&payload.app_id)
        .bind(line.employee_id)
        .bind(&line.employee_name)
        .bind(line.project_id)
        .bind(&line.project_name)
        .bind(payload.period_start)
        .bind(payload.period_end)
        .bind(line.total_minutes)
        .bind(line.hourly_rate_minor)
        .bind(line.amount_minor)
        .bind(&line.currency)
        .bind(&line.description)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;

    Ok(BillableTimeImportResult {
        export_id: payload.export_id,
        lines_imported: payload.lines.len(),
        total_amount_minor: payload.total_amount_minor,
        already_imported: false,
    })
}

/// Ensure the import tracking table exists.
async fn ensure_import_table(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS ar_tk_billable_imports (
            id              BIGSERIAL PRIMARY KEY,
            export_id       UUID NOT NULL,
            app_id          TEXT NOT NULL,
            employee_id     UUID NOT NULL,
            employee_name   TEXT NOT NULL,
            project_id      UUID NOT NULL,
            project_name    TEXT NOT NULL,
            period_start    DATE NOT NULL,
            period_end      DATE NOT NULL,
            total_minutes   INT NOT NULL,
            hourly_rate_minor BIGINT NOT NULL,
            amount_minor    BIGINT NOT NULL,
            currency        TEXT NOT NULL DEFAULT 'USD',
            description     TEXT,
            invoiced        BOOLEAN NOT NULL DEFAULT FALSE,
            created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )
        "#,
    )
    .execute(pool)
    .await?;

    // Index for idempotency lookups
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_ar_tk_billable_imports_export_id \
         ON ar_tk_billable_imports (export_id)",
    )
    .execute(pool)
    .await?;

    Ok(())
}
