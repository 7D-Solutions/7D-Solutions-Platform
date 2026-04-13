//! Period Close Execution
//!
//! Atomic close command orchestrator for accounting periods.
//! Coordinates validation, FX revaluation, snapshot sealing, and DB update.

use super::period_close_snapshot::create_close_snapshot;
use super::period_close_validation::{
    check_close_checklist_gate, has_blocking_errors, validate_period_can_close, PeriodCloseError,
};
use crate::contracts::period_close_v1::{
    CloseStatus, ValidationIssue, ValidationReport, ValidationSeverity,
};
use chrono::{DateTime, NaiveDate, Utc};
use platform_audit::schema::{MutationClass, WriteAuditRequest};
use platform_audit::writer::AuditWriter;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

/// Response from close_period operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClosePeriodResult {
    /// Period ID that was closed (or attempted to close)
    pub period_id: Uuid,

    /// Tenant ID
    pub tenant_id: String,

    /// Whether the close succeeded
    pub success: bool,

    /// Close status (if successful)
    pub close_status: Option<CloseStatus>,

    /// Validation report (if close failed validation)
    pub validation_report: Option<ValidationReport>,

    /// Timestamp when operation completed
    pub timestamp: DateTime<Utc>,
}

/// Period data with close fields for idempotency check
#[derive(Debug, sqlx::FromRow)]
#[allow(dead_code)]
struct PeriodForClose {
    pub id: Uuid,
    pub tenant_id: String,
    pub period_start: chrono::NaiveDate,
    pub period_end: chrono::NaiveDate,
    pub closed_at: Option<DateTime<Utc>>,
    pub closed_by: Option<String>,
    pub close_reason: Option<String>,
    pub close_hash: Option<String>,
    pub close_requested_at: Option<DateTime<Utc>>,
}

/// Atomically close an accounting period
///
/// This function implements the complete period close workflow:
/// 1. Locks the period row (FOR UPDATE) to prevent concurrent closes
/// 2. Checks if already closed (idempotency)
/// 3. Runs pre-close validation defensively
///    - Revalue foreign-currency balances (Phase 23a)
/// 4. Creates sealed snapshot with hash (includes revaluation entries)
/// 5. Updates period with close fields
///
/// All operations occur in a single database transaction for atomicity.
///
/// **Idempotency:** If period.closed_at is already set, returns existing close status
/// without mutation. This is determined AFTER acquiring the row lock to prevent race conditions.
///
/// **Locking:** Uses FOR UPDATE to acquire row-level lock at transaction start.
/// This prevents two concurrent close requests from both seeing closed_at=NULL.
///
/// # Arguments
/// * `pool` - Database connection pool
/// * `tenant_id` - Tenant identifier
/// * `period_id` - Accounting period UUID
/// * `closed_by` - User or system identifier performing the close
/// * `close_reason` - Optional reason/notes for closing the period
/// * `dlq_validation_enabled` - Enable DLQ validation (default: false)
/// * `reporting_currency` - Reporting currency for FX revaluation (e.g. "USD")
///
/// # Returns
/// ClosePeriodResult with success status, close status, or validation errors
pub async fn close_period(
    pool: &PgPool,
    tenant_id: &str,
    period_id: Uuid,
    closed_by: &str,
    close_reason: Option<&str>,
    dlq_validation_enabled: bool,
    reporting_currency: &str,
) -> Result<ClosePeriodResult, PeriodCloseError> {
    // BEGIN transaction
    let mut tx = pool.begin().await?;

    // ========================================
    // STEP 1: Lock period row with FOR UPDATE (BEFORE any other operations)
    // ========================================
    // This prevents race conditions where two concurrent close requests
    // both see closed_at=NULL and both proceed to close.
    let period = sqlx::query_as::<_, PeriodForClose>(
        r#"
        SELECT id, tenant_id, period_start, period_end,
               closed_at, closed_by, close_reason, close_hash, close_requested_at
        FROM accounting_periods
        WHERE id = $1 AND tenant_id = $2
        FOR UPDATE
        "#,
    )
    .bind(period_id)
    .bind(tenant_id)
    .fetch_optional(&mut *tx)
    .await?;

    let period = match period {
        Some(p) => p,
        None => {
            tx.rollback().await?;
            return Ok(ClosePeriodResult {
                period_id,
                tenant_id: tenant_id.to_string(),
                success: false,
                close_status: None,
                validation_report: Some(ValidationReport {
                    issues: vec![ValidationIssue {
                        severity: ValidationSeverity::Error,
                        code: "PERIOD_NOT_FOUND".to_string(),
                        message: format!("Period {} not found for tenant {}", period_id, tenant_id),
                        metadata: None,
                    }],
                }),
                timestamp: Utc::now(),
            });
        }
    };

    // ========================================
    // STEP 2: Check idempotency (AFTER acquiring lock)
    // ========================================
    // If period is already closed, return existing close status without mutation.
    // This check happens AFTER the lock to prevent TOCTOU (time-of-check-time-of-use) race.
    if let Some(closed_at) = period.closed_at {
        tx.commit().await?;

        return Ok(ClosePeriodResult {
            period_id,
            tenant_id: tenant_id.to_string(),
            success: true,
            close_status: Some(CloseStatus::Closed {
                closed_at,
                closed_by: period.closed_by.clone().unwrap_or_default(),
                close_reason: period.close_reason.clone(),
                close_hash: period.close_hash.clone().unwrap_or_default(),
                requested_at: period.close_requested_at,
            }),
            validation_report: None,
            timestamp: Utc::now(),
        });
    }

    // ========================================
    // STEP 2b: Check pre-close checklist gate (Phase 31, bd-bfa3)
    // ========================================
    // If checklist items exist, ALL must be complete/waived and at least
    // one approval signoff must be recorded. If no items → gate passes.
    let gate_issues = check_close_checklist_gate(&mut tx, tenant_id, period_id).await?;
    if !gate_issues.is_empty() {
        tx.rollback().await?;

        return Ok(ClosePeriodResult {
            period_id,
            tenant_id: tenant_id.to_string(),
            success: false,
            close_status: None,
            validation_report: Some(ValidationReport {
                issues: gate_issues,
            }),
            timestamp: Utc::now(),
        });
    }

    // ========================================
    // STEP 3: Run pre-close validation (defensive)
    // ========================================
    // Always re-validate before close, even if client pre-validated.
    // ChatGPT guardrail: validation MUST re-run on every close attempt.
    let validation_report =
        validate_period_can_close(&mut tx, tenant_id, period_id, dlq_validation_enabled).await?;

    if has_blocking_errors(&validation_report) {
        tx.rollback().await?;

        return Ok(ClosePeriodResult {
            period_id,
            tenant_id: tenant_id.to_string(),
            success: false,
            close_status: None,
            validation_report: Some(validation_report),
            timestamp: Utc::now(),
        });
    }

    // ========================================
    // STEP 3b: FX Revaluation (Phase 23a, bd-1yu)
    // ========================================
    // Revalue foreign-currency balances BEFORE the snapshot so revaluation
    // journal entries are included in the sealed hash.
    let reval_result = super::fx_revaluation_service::revalue_foreign_balances(
        &mut tx,
        tenant_id,
        period_id,
        period.period_start,
        period.period_end,
        reporting_currency,
    )
    .await?;

    if let Some(ref entry_id) = reval_result.journal_entry_id {
        tracing::info!(
            tenant_id = %tenant_id,
            period_id = %period_id,
            journal_entry_id = %entry_id,
            adjustment_count = reval_result.adjustments.len(),
            "FX revaluation posted during period close"
        );
    }

    // ========================================
    // STEP 4: Create sealed snapshot with hash
    // ========================================
    let snapshot = create_close_snapshot(&mut tx, tenant_id, period_id).await?;

    // ========================================
    // STEP 5: Update accounting_periods with close fields
    // ========================================
    let now = Utc::now();

    sqlx::query(
        r#"
        UPDATE accounting_periods
        SET close_requested_at = COALESCE(close_requested_at, $1),
            closed_at = $2,
            closed_by = $3,
            close_reason = $4,
            close_hash = $5
        WHERE id = $6 AND tenant_id = $7
        "#,
    )
    .bind(now)
    .bind(now)
    .bind(closed_by)
    .bind(close_reason)
    .bind(&snapshot.close_hash)
    .bind(period_id)
    .bind(tenant_id)
    .execute(&mut *tx)
    .await?;

    // Audit: record period close inside the same transaction
    let audit_req = WriteAuditRequest::new(
        Uuid::nil(),
        "system".to_string(),
        "ClosePeriod".to_string(),
        MutationClass::StateTransition,
        "AccountingPeriod".to_string(),
        period_id.to_string(),
    );
    AuditWriter::write_in_tx(&mut tx, audit_req).await
        .map_err(|e| match e {
            platform_audit::writer::AuditWriterError::Database(db) => PeriodCloseError::Database(db),
            platform_audit::writer::AuditWriterError::InvalidRequest(msg) => {
                PeriodCloseError::Database(sqlx::Error::Protocol(msg))
            }
        })?;

    // ========================================
    // STEP 6: COMMIT transaction
    // ========================================
    tx.commit().await?;

    Ok(ClosePeriodResult {
        period_id,
        tenant_id: tenant_id.to_string(),
        success: true,
        close_status: Some(CloseStatus::Closed {
            closed_at: now,
            closed_by: closed_by.to_string(),
            close_reason: close_reason.map(|s| s.to_string()),
            close_hash: snapshot.close_hash,
            requested_at: Some(now),
        }),
        validation_report: None,
        timestamp: now,
    })
}

// ============================================================
// Close Calendar — read-only status reflecting actual GL state
// ============================================================

/// Close calendar entry with actual GL close state merged in
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloseCalendarEntry {
    pub id: Uuid,
    pub tenant_id: String,
    pub period_id: Uuid,
    pub period_start: NaiveDate,
    pub period_end: NaiveDate,
    pub expected_close_date: NaiveDate,
    pub owner_role: String,
    pub reminder_offset_days: Vec<i32>,
    pub overdue_reminder_interval_days: i32,
    pub gl_closed: bool,
    pub gl_closed_at: Option<DateTime<Utc>>,
    pub notes: Option<String>,
}

/// Row type for the calendar + period join query
#[derive(Debug, sqlx::FromRow)]
struct CalendarPeriodRow {
    id: Uuid,
    tenant_id: String,
    period_id: Uuid,
    period_start: NaiveDate,
    period_end: NaiveDate,
    expected_close_date: NaiveDate,
    owner_role: String,
    reminder_offset_days: Vec<i32>,
    overdue_reminder_interval_days: i32,
    notes: Option<String>,
    closed_at: Option<DateTime<Utc>>,
}

/// Get close calendar entries for a tenant, with actual GL close state.
///
/// Joins close_calendar with accounting_periods to reflect whether each
/// period is actually closed in GL. This is a read-only check — the
/// calendar never stores its own open/closed status.
pub async fn get_close_calendar_status(
    pool: &PgPool,
    tenant_id: &str,
) -> Result<Vec<CloseCalendarEntry>, PeriodCloseError> {
    let rows = sqlx::query_as::<_, CalendarPeriodRow>(
        r#"
        SELECT cc.id, cc.tenant_id, cc.period_id,
               ap.period_start, ap.period_end,
               cc.expected_close_date, cc.owner_role,
               cc.reminder_offset_days, cc.overdue_reminder_interval_days,
               cc.notes, ap.closed_at
        FROM close_calendar cc
        JOIN accounting_periods ap ON ap.id = cc.period_id AND ap.tenant_id = cc.tenant_id
        WHERE cc.tenant_id = $1
        ORDER BY cc.expected_close_date ASC
        "#,
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| CloseCalendarEntry {
            id: r.id,
            tenant_id: r.tenant_id,
            period_id: r.period_id,
            period_start: r.period_start,
            period_end: r.period_end,
            expected_close_date: r.expected_close_date,
            owner_role: r.owner_role,
            reminder_offset_days: r.reminder_offset_days,
            overdue_reminder_interval_days: r.overdue_reminder_interval_days,
            gl_closed: r.closed_at.is_some(),
            gl_closed_at: r.closed_at,
            notes: r.notes,
        })
        .collect())
}
