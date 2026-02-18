//! Period Close Validation
//!
//! Pre-close validation engine for accounting periods.
//! Checks that a period can be safely closed before initiating the close operation.

use crate::contracts::period_close_v1::{ValidationIssue, ValidationReport, ValidationSeverity};
use chrono::DateTime;
use chrono::Utc;
use sqlx::{Postgres, Transaction};
use thiserror::Error;
use uuid::Uuid;

/// Errors that can occur during period close operations
#[derive(Debug, Error)]
pub enum PeriodCloseError {
    #[error("Period not found: {0}")]
    PeriodNotFound(Uuid),

    #[error("Period already closed: {0}")]
    PeriodAlreadyClosed(Uuid),

    #[error("Validation failed: {0}")]
    ValidationFailed(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Hash verification failed - computed: {computed}, expected: {expected}")]
    HashMismatch { computed: String, expected: String },

    #[error("FX revaluation failed: {0}")]
    FxRevaluation(#[from] super::fx_revaluation_service::FxRevaluationError),
}

/// Accounting period data for validation
#[derive(Debug, sqlx::FromRow)]
pub(crate) struct PeriodData {
    pub id: Uuid,
    pub tenant_id: String,
    pub period_start: chrono::NaiveDate,
    pub period_end: chrono::NaiveDate,
    pub closed_at: Option<DateTime<Utc>>,
    pub close_requested_at: Option<DateTime<Utc>>,
}

/// Unbalanced journal check result
#[derive(Debug, sqlx::FromRow)]
struct UnbalancedJournalCheck {
    pub unbalanced_count: i64,
}

/// DLQ pending retry check result
#[derive(Debug, sqlx::FromRow)]
struct DlqPendingRetryCheck {
    pub pending_count: i64,
}

/// Check for pending DLQ entries for posting-related subjects (optional validation)
///
/// This is a bounded query that checks the failed_events table for:
/// - Tenant-scoped entries only
/// - Posting-related subjects: "gl.events.posting.requested"
///
/// **Note:** This validation is tenant-scoped and bounded - it only queries
/// the GL module's own DLQ table, not any cross-module dependencies.
///
/// # Arguments
/// * `tx` - Database transaction
/// * `tenant_id` - Tenant identifier
///
/// # Returns
/// Count of pending DLQ entries for this tenant
async fn check_pending_dlq_entries(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
) -> Result<i64, PeriodCloseError> {
    let result = sqlx::query_as::<_, DlqPendingRetryCheck>(
        r#"
        SELECT COUNT(*) as pending_count
        FROM failed_events
        WHERE tenant_id = $1
          AND subject = 'gl.events.posting.requested'
        "#,
    )
    .bind(tenant_id)
    .fetch_one(&mut **tx)
    .await?;

    Ok(result.pending_count)
}

/// Validate if a period can be closed (pre-close validation)
///
/// **Mandatory validations:**
/// 1. Period exists (tenant-scoped)
/// 2. Period not already closed (closed_at IS NULL)
/// 3. No unbalanced journal entries in the period
///
/// **Optional validations** (feature-gated via config):
/// - Tenant DLQ empty for posting-related subjects (bd-31u)
///
/// Returns a structured ValidationReport with errors/warnings.
/// If any ERRORS exist, close should be blocked (can_close=false).
///
/// # Arguments
/// * `tx` - Database transaction (for consistency with close operation)
/// * `tenant_id` - Tenant identifier
/// * `period_id` - Accounting period UUID
/// * `dlq_validation_enabled` - Optional flag to enable DLQ validation (default: false)
///
/// # Returns
/// ValidationReport with issues (empty if validation passes)
pub async fn validate_period_can_close(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    period_id: Uuid,
    dlq_validation_enabled: bool,
) -> Result<ValidationReport, PeriodCloseError> {
    let mut issues = Vec::new();

    // ========================================
    // MANDATORY VALIDATION 1: Period exists (tenant-scoped)
    // ========================================
    let period_data = sqlx::query_as::<_, PeriodData>(
        r#"
        SELECT id, tenant_id, period_start, period_end, closed_at, close_requested_at
        FROM accounting_periods
        WHERE id = $1 AND tenant_id = $2
        "#,
    )
    .bind(period_id)
    .bind(tenant_id)
    .fetch_optional(&mut **tx)
    .await?;

    let period = match period_data {
        Some(p) => p,
        None => {
            // CRITICAL ERROR: Period not found
            issues.push(ValidationIssue {
                severity: ValidationSeverity::Error,
                code: "PERIOD_NOT_FOUND".to_string(),
                message: format!(
                    "Period {} not found for tenant {}",
                    period_id, tenant_id
                ),
                metadata: None,
            });

            // Return early - cannot perform further validations
            return Ok(ValidationReport { issues });
        }
    };

    // ========================================
    // MANDATORY VALIDATION 2: Period not already closed
    // ========================================
    if period.closed_at.is_some() {
        issues.push(ValidationIssue {
            severity: ValidationSeverity::Error,
            code: "PERIOD_ALREADY_CLOSED".to_string(),
            message: format!(
                "Period {} is already closed at {}",
                period_id,
                period
                    .closed_at
                    .unwrap()
                    .to_rfc3339()
            ),
            metadata: Some(serde_json::json!({
                "closed_at": period.closed_at.unwrap().to_rfc3339(),
            })),
        });
    }

    // ========================================
    // MANDATORY VALIDATION 3: No unbalanced journal entries
    // ========================================
    // Query for journal entries in this period where total debits != total credits
    // This is a DEFENSIVE check (should never happen due to posting validation)
    let unbalanced = sqlx::query_as::<_, UnbalancedJournalCheck>(
        r#"
        SELECT COUNT(*) as unbalanced_count
        FROM journal_entries je
        WHERE je.tenant_id = $1
          AND je.posted_at::DATE >= $2
          AND je.posted_at::DATE <= $3
          AND je.id IN (
              SELECT jl.journal_entry_id
              FROM journal_lines jl
              WHERE jl.journal_entry_id = je.id
              GROUP BY jl.journal_entry_id
              HAVING COALESCE(SUM(jl.debit_minor), 0) != COALESCE(SUM(jl.credit_minor), 0)
          )
        "#,
    )
    .bind(tenant_id)
    .bind(period.period_start)
    .bind(period.period_end)
    .fetch_one(&mut **tx)
    .await?;

    if unbalanced.unbalanced_count > 0 {
        issues.push(ValidationIssue {
            severity: ValidationSeverity::Error,
            code: "UNBALANCED_ENTRIES".to_string(),
            message: format!(
                "Period has {} unbalanced journal entries - debits do not equal credits",
                unbalanced.unbalanced_count
            ),
            metadata: Some(serde_json::json!({
                "unbalanced_count": unbalanced.unbalanced_count,
            })),
        });
    }

    // ========================================
    // OPTIONAL VALIDATION: DLQ Empty Check (bd-31u)
    // ========================================
    // If enabled via config, check for pending DLQ entries for posting-related subjects.
    // This is a bounded, tenant-scoped query within GL boundaries.
    if dlq_validation_enabled {
        let pending_dlq_count = check_pending_dlq_entries(tx, tenant_id).await?;

        if pending_dlq_count > 0 {
            issues.push(ValidationIssue {
                severity: ValidationSeverity::Error,
                code: "PENDING_DLQ_ENTRIES".to_string(),
                message: format!(
                    "Period cannot be closed: tenant has {} pending DLQ entries for posting-related subjects",
                    pending_dlq_count
                ),
                metadata: Some(serde_json::json!({
                    "pending_dlq_count": pending_dlq_count,
                    "subject_filter": "gl.events.posting.requested",
                })),
            });
        }
    }

    Ok(ValidationReport { issues })
}

/// Helper: Check if validation report has blocking errors
///
/// Returns true if any issues have severity=Error (blocks close operation)
pub fn has_blocking_errors(report: &ValidationReport) -> bool {
    report
        .issues
        .iter()
        .any(|issue| matches!(issue.severity, ValidationSeverity::Error))
}

// ============================================================
// Pre-Close Checklist Gate (Phase 31, bd-bfa3)
// ============================================================

/// Checklist gate status for pre-close validation
#[derive(Debug, sqlx::FromRow)]
struct ChecklistGateCounts {
    pub total_items: i64,
    pub pending_items: i64,
    pub approval_count: i64,
}

/// Check the pre-close checklist gate for a period.
///
/// Gate logic:
/// - If no checklist items exist → gate passes (no checklist configured)
/// - If checklist items exist → ALL must be 'complete' or 'waived'
/// - If checklist items exist → at least one approval signoff required
///
/// Returns validation issues (empty if gate passes).
pub async fn check_close_checklist_gate(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    period_id: Uuid,
) -> Result<Vec<ValidationIssue>, PeriodCloseError> {
    let mut issues = Vec::new();

    let counts = sqlx::query_as::<_, ChecklistGateCounts>(
        r#"
        SELECT
            (SELECT COUNT(*) FROM close_checklist_items
             WHERE tenant_id = $1 AND period_id = $2) AS total_items,
            (SELECT COUNT(*) FROM close_checklist_items
             WHERE tenant_id = $1 AND period_id = $2 AND status = 'pending') AS pending_items,
            (SELECT COUNT(*) FROM close_approvals
             WHERE tenant_id = $1 AND period_id = $2) AS approval_count
        "#,
    )
    .bind(tenant_id)
    .bind(period_id)
    .fetch_one(&mut **tx)
    .await?;

    // No checklist configured → gate passes
    if counts.total_items == 0 {
        return Ok(issues);
    }

    if counts.pending_items > 0 {
        issues.push(ValidationIssue {
            severity: ValidationSeverity::Error,
            code: "CHECKLIST_INCOMPLETE".to_string(),
            message: format!(
                "Pre-close checklist has {} pending item(s) out of {} total",
                counts.pending_items, counts.total_items
            ),
            metadata: Some(serde_json::json!({
                "total_items": counts.total_items,
                "pending_items": counts.pending_items,
            })),
        });
    }

    if counts.approval_count == 0 {
        issues.push(ValidationIssue {
            severity: ValidationSeverity::Error,
            code: "APPROVAL_MISSING".to_string(),
            message: "No approval signoffs recorded — at least one required when checklist is configured".to_string(),
            metadata: None,
        });
    }

    Ok(issues)
}
