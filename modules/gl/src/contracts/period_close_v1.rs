//! Period Close Workflow V1 Contract Types
//!
//! Phase 13: Defines API contracts for period close lifecycle operations:
//! - Validate Close: Pre-flight validation before closing a period
//! - Close Period: Atomically close a period with snapshot + hash
//! - Close Status: Query the close status and validation report
//!
//! All operations are tenant-scoped and idempotent.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ============================================================
// Validate Close Endpoint: POST /api/gl/periods/{period_id}/validate-close
// ============================================================

/// Request to validate if a period can be closed
///
/// Pre-flight check before actual close operation.
/// Does NOT modify period state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ValidateCloseRequest {
    /// Tenant ID for multi-tenancy isolation
    pub tenant_id: String,
}

/// Response from validate-close operation
///
/// Returns structured validation report with errors/warnings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ValidateCloseResponse {
    /// Period ID that was validated
    pub period_id: Uuid,

    /// Tenant ID
    pub tenant_id: String,

    /// Overall validation result
    pub can_close: bool,

    /// Structured validation report (empty if can_close=true)
    pub validation_report: ValidationReport,

    /// Timestamp when validation was performed
    pub validated_at: DateTime<Utc>,
}

// ============================================================
// Close Period Endpoint: POST /api/gl/periods/{period_id}/close
// ============================================================

/// Request to close an accounting period
///
/// Idempotent operation - if already closed, returns existing close status.
/// Atomically: validates, creates snapshot, computes hash, sets closed_at.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ClosePeriodRequest {
    /// Tenant ID for multi-tenancy isolation
    pub tenant_id: String,

    /// User or system identifier performing the close
    pub closed_by: String,

    /// Optional reason/notes for closing the period (for audit trail)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub close_reason: Option<String>,
}

/// Response from close operation
///
/// Returns close status including snapshot hash for audit verification.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ClosePeriodResponse {
    /// Period ID that was closed
    pub period_id: Uuid,

    /// Tenant ID
    pub tenant_id: String,

    /// Whether close succeeded
    pub success: bool,

    /// Close status details (if successful)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub close_status: Option<CloseStatus>,

    /// Validation report (if close failed validation)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation_report: Option<ValidationReport>,

    /// Timestamp of response
    pub timestamp: DateTime<Utc>,
}

// ============================================================
// Close Status Endpoint: GET /api/gl/periods/{period_id}/close-status
// ============================================================

/// Request to query period close status
///
/// Read-only query for current close workflow state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CloseStatusRequest {
    /// Tenant ID for multi-tenancy isolation
    pub tenant_id: String,
}

/// Response with period close status
///
/// Returns current state of period in close workflow.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CloseStatusResponse {
    /// Period ID
    pub period_id: Uuid,

    /// Tenant ID
    pub tenant_id: String,

    /// Period date range
    pub period_start: String,  // YYYY-MM-DD format
    pub period_end: String,    // YYYY-MM-DD format

    /// Close status details
    pub close_status: CloseStatus,

    /// Timestamp of response
    pub timestamp: DateTime<Utc>,
}

// ============================================================
// Shared Domain Types
// ============================================================

/// Close workflow status
///
/// Represents the current state of a period in the close lifecycle.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "state", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CloseStatus {
    /// Period is open - can post and reverse transactions
    Open,

    /// Close has been requested but not yet completed
    CloseRequested {
        /// When close was requested
        requested_at: DateTime<Utc>,
    },

    /// Period is permanently closed
    Closed {
        /// When period was closed
        closed_at: DateTime<Utc>,

        /// Who closed the period
        closed_by: String,

        /// Optional close reason
        #[serde(skip_serializing_if = "Option::is_none")]
        close_reason: Option<String>,

        /// SHA-256 hash of period snapshot for tamper detection
        close_hash: String,

        /// When close was initially requested (may be same as closed_at)
        #[serde(skip_serializing_if = "Option::is_none")]
        requested_at: Option<DateTime<Utc>>,
    },
}

/// Structured validation report
///
/// Machine-readable validation results with severity levels.
/// Empty if validation passes (can_close=true).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ValidationReport {
    /// Validation issues grouped by severity
    pub issues: Vec<ValidationIssue>,
}

/// Individual validation issue
///
/// Structured error/warning with stable code for client handling.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ValidationIssue {
    /// Severity level
    pub severity: ValidationSeverity,

    /// Stable error code for programmatic handling
    /// Examples: "PERIOD_NOT_FOUND", "UNBALANCED_ENTRIES", "PENDING_TRANSACTIONS"
    pub code: String,

    /// Human-readable message
    pub message: String,

    /// Optional metadata for additional context
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// Validation issue severity levels
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ValidationSeverity {
    /// Blocks close operation
    Error,

    /// Should be addressed but doesn't block close
    Warning,

    /// Informational only
    Info,
}

// ============================================================
// Idempotency Semantics
// ============================================================

/// Idempotency for close operation is deterministic via closed_at field.
///
/// **ChatGPT Guardrail:**
/// - If period.closed_at IS NOT NULL → return existing close status (no mutation)
/// - Else → execute close transaction
///
/// No client-provided idempotency keys needed - period state is source of truth.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_close_status_serialization() {
        let status = CloseStatus::Closed {
            closed_at: Utc::now(),
            closed_by: "admin".to_string(),
            close_reason: Some("Month-end close".to_string()),
            close_hash: "abc123".to_string(),
            requested_at: None,
        };

        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("CLOSED"));
        assert!(json.contains("admin"));
    }

    #[test]
    fn test_validation_issue_creation() {
        let issue = ValidationIssue {
            severity: ValidationSeverity::Error,
            code: "UNBALANCED_ENTRIES".to_string(),
            message: "Period has unbalanced journal entries".to_string(),
            metadata: None,
        };

        assert_eq!(issue.code, "UNBALANCED_ENTRIES");
        assert_eq!(issue.severity, ValidationSeverity::Error);
    }

    #[test]
    fn test_empty_validation_report() {
        let report = ValidationReport { issues: vec![] };
        assert!(report.issues.is_empty());
    }
}
