//! Payments Retry Window Discipline Module (Phase 15 - bd-1it)
//!
//! **Fixed Retry Windows:** This module implements deterministic retry scheduling
//! for payment collection attempts.
//!
//! **Retry Schedule:**
//! - Attempt 0: First attempt date (immediate)
//! - Attempt 1: First attempt date + 3 days
//! - Attempt 2: First attempt date + 7 days
//!
//! **Critical Invariant (ChatGPT):**
//! Exactly one attempt per window, enforced by attempt_no derived from window index.
//! No configurability - windows are hard-coded for deterministic behavior.
//!
//! **UNKNOWN Blocking Protocol (bd-2uw):**
//! Payment attempts with status='unknown' are EXCLUDED from retry scheduling.
//! UNKNOWN indicates webhook ambiguity - customer is not at fault.
//! Reconciliation workflow (bd-2uw) must resolve UNKNOWN before retry can proceed.
//!
//! **Integration (bd-2wtz Module Isolation):**
//! - Uses payment_attempts UNIQUE(app_id, payment_id, attempt_no) from bd-7gl
//! - Calls payments::lifecycle guards from bd-3lm
//! - Respects UNKNOWN protocol from bd-2uw
//! - Uses first attempt's attempted_at as retry anchor (removed AR cross-module dependency)

use chrono::{NaiveDate, Utc};
use sqlx::PgPool;
use std::fmt;
use uuid::Uuid;

// ============================================================================
// Retry Window Configuration
// ============================================================================

/// Fixed retry windows (no configurability)
/// These mirror AR retry windows (bd-8ev) for consistency across modules
pub mod windows {
    /// Attempt 0: Immediate (due date)
    pub const ATTEMPT_0_OFFSET_DAYS: i64 = 0;

    /// Attempt 1: +3 days after due date
    pub const ATTEMPT_1_OFFSET_DAYS: i64 = 3;

    /// Attempt 2: +7 days after due date
    pub const ATTEMPT_2_OFFSET_DAYS: i64 = 7;

    /// Maximum number of retry attempts
    pub const MAX_ATTEMPTS: i32 = 3;
}

// ============================================================================
// Error Types
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetryError {
    /// Payment attempt not found
    AttemptNotFound(Uuid),
    /// Payment has no due date set (AR invoice missing due_at)
    NoDueDate {
        payment_id: Uuid,
        invoice_id: String,
    },
    /// Payment not eligible for retry (wrong status)
    NotEligible {
        payment_id: Uuid,
        current_status: String,
        reason: String,
    },
    /// No more retry windows available (max attempts reached)
    MaxAttemptsReached {
        payment_id: Uuid,
        attempt_count: i32,
    },
    /// Database error
    DatabaseError(String),
}

impl fmt::Display for RetryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AttemptNotFound(id) => write!(f, "Payment attempt not found: {}", id),
            Self::NoDueDate {
                payment_id,
                invoice_id,
            } => write!(
                f,
                "Payment {} (invoice {}) has no due date",
                payment_id, invoice_id
            ),
            Self::NotEligible {
                payment_id,
                current_status,
                reason,
            } => write!(
                f,
                "Payment {} not eligible for retry (status: {}): {}",
                payment_id, current_status, reason
            ),
            Self::MaxAttemptsReached {
                payment_id,
                attempt_count,
            } => write!(
                f,
                "Payment {} has reached max attempts ({})",
                payment_id, attempt_count
            ),
            Self::DatabaseError(msg) => write!(f, "Database error: {}", msg),
        }
    }
}

impl std::error::Error for RetryError {}

impl From<sqlx::Error> for RetryError {
    fn from(e: sqlx::Error) -> Self {
        Self::DatabaseError(e.to_string())
    }
}

// ============================================================================
// Retry Window Calculation
// ============================================================================

/// Calculate retry window dates for a payment
///
/// **Returns:** Array of 3 dates: [attempt_0_date, attempt_1_date, attempt_2_date]
///
/// **Example:**
/// ```rust,ignore
/// let due_date = NaiveDate::from_ymd_opt(2026, 2, 15).unwrap();
/// let windows = calculate_retry_windows(due_date);
/// // windows = [2026-02-15, 2026-02-18, 2026-02-22]
/// ```
pub fn calculate_retry_windows(due_date: NaiveDate) -> [NaiveDate; 3] {
    [
        due_date,                                                            // Attempt 0: immediate
        due_date + chrono::Days::new(windows::ATTEMPT_1_OFFSET_DAYS as u64), // Attempt 1: +3 days
        due_date + chrono::Days::new(windows::ATTEMPT_2_OFFSET_DAYS as u64), // Attempt 2: +7 days
    ]
}

/// Determine which retry window we're currently in based on today's date
///
/// **Returns:**
/// - Some(attempt_no) if we're in a retry window
/// - None if no window is active yet or all windows have passed
///
/// **Example:**
/// ```rust,ignore
/// let due_date = NaiveDate::from_ymd_opt(2026, 2, 15).unwrap();
/// let today = NaiveDate::from_ymd_opt(2026, 2, 18).unwrap();
/// let window = determine_current_window(due_date, today);
/// assert_eq!(window, Some(1)); // In attempt 1 window (+3 days)
/// ```
pub fn determine_current_window(due_date: NaiveDate, today: NaiveDate) -> Option<i32> {
    let windows = calculate_retry_windows(due_date);

    // Check if we're in or past each window
    for (attempt_no, window_date) in windows.iter().enumerate() {
        if today >= *window_date {
            // Check if next window is also active
            if attempt_no < 2 && today >= windows[attempt_no + 1] {
                continue; // Skip to next window
            }
            return Some(attempt_no as i32);
        }
    }

    None
}

/// Check if payment attempt is eligible for retry based on current status
///
/// **Eligible statuses:**
/// - "attempting" (first attempt)
/// - "failed_retry" (retry attempts)
///
/// **Ineligible statuses:**
/// - "succeeded" (terminal - success)
/// - "failed_final" (terminal - max retries exhausted)
/// - "unknown" (BLOCKED - waiting for reconciliation via bd-2uw)
///
/// **UNKNOWN Blocking Protocol:**
/// Payments with status='unknown' are EXCLUDED from retry scheduling.
/// This protects customers from being charged multiple times for ambiguous payment results.
/// Reconciliation workflow (bd-2uw) must resolve UNKNOWN → SUCCEEDED/FAILED_* before retry.
pub fn is_eligible_for_retry(status: &str) -> bool {
    matches!(status, "attempting" | "failed_retry")
}

// ============================================================================
// Retry Scheduling
// ============================================================================

/// Get payment attempts eligible for retry in current window
///
/// **Returns:** List of (payment_id, attempt_no) tuples that should be retried today
///
/// **Logic:**
/// 1. Fetch payment attempts with eligible status (attempting, failed_retry)
/// 2. EXCLUDE attempts with status='unknown' (UNKNOWN protocol - bd-2uw)
/// 3. Use first attempt's attempted_at date as retry anchor (removed cross-module AR dependency)
/// 4. Calculate which retry window they're in based on first attempted_at
/// 5. Check if attempt already exists for that window
/// 6. Return payments that need attempt in current window
///
/// **UNKNOWN Blocking:**
/// This function explicitly filters out status='unknown' to prevent retry scheduling
/// for payments waiting reconciliation. Customer is not at fault for webhook ambiguity.
///
/// **Module Isolation (bd-2wtz):**
/// Uses attempted_at from payment_attempts (attempt_no=0) instead of joining to AR schema.
/// Payments module owns retry scheduling logic without cross-module dependencies.
pub async fn get_payments_for_retry(
    pool: &PgPool,
    app_id: &str,
) -> Result<Vec<(Uuid, i32)>, RetryError> {
    // Fetch payment IDs with first attempt date (attempt_no=0)
    // CRITICAL: EXCLUDE status='unknown' (UNKNOWN protocol - bd-2uw)
    // Use attempted_at from first attempt as retry anchor (no AR dependency)
    let payments_with_first_attempt: Vec<(Uuid, NaiveDate)> = sqlx::query_as(
        "SELECT DISTINCT ON (payment_id) payment_id, attempted_at::date
         FROM payment_attempts
         WHERE app_id = $1
           AND attempt_no = 0
           AND payment_id IN (
               SELECT DISTINCT payment_id
               FROM payment_attempts
               WHERE app_id = $1
                 AND status::text IN ('attempting', 'failed_retry')
                 AND status::text != 'unknown'  -- UNKNOWN blocking protocol
           )
         ORDER BY payment_id, attempted_at ASC",
    )
    .bind(app_id)
    .fetch_all(pool)
    .await?;

    let today = Utc::now().date_naive();
    let mut retry_list = Vec::new();

    for (payment_id, first_attempt_date) in payments_with_first_attempt {
        // Determine current retry window based on first attempt date
        let current_window = match determine_current_window(first_attempt_date, today) {
            Some(w) => w,
            None => continue, // Not in any window yet
        };

        // Check if attempt already exists for this window
        let attempt_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(
                SELECT 1 FROM payment_attempts
                WHERE app_id = $1 AND payment_id = $2 AND attempt_no = $3
            )",
        )
        .bind(app_id)
        .bind(payment_id)
        .bind(current_window)
        .fetch_one(pool)
        .await?;

        if !attempt_exists {
            retry_list.push((payment_id, current_window));
        }
    }

    Ok(retry_list)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn test_calculate_retry_windows() {
        let due_date = NaiveDate::from_ymd_opt(2026, 2, 15).unwrap();
        let windows = calculate_retry_windows(due_date);

        assert_eq!(windows[0], NaiveDate::from_ymd_opt(2026, 2, 15).unwrap());
        assert_eq!(windows[1], NaiveDate::from_ymd_opt(2026, 2, 18).unwrap());
        assert_eq!(windows[2], NaiveDate::from_ymd_opt(2026, 2, 22).unwrap());
    }

    #[test]
    fn test_determine_current_window_attempt_0() {
        let due_date = NaiveDate::from_ymd_opt(2026, 2, 15).unwrap();

        // On due date
        let today = NaiveDate::from_ymd_opt(2026, 2, 15).unwrap();
        assert_eq!(determine_current_window(due_date, today), Some(0));

        // Day after due date (still in attempt 0 window)
        let today = NaiveDate::from_ymd_opt(2026, 2, 16).unwrap();
        assert_eq!(determine_current_window(due_date, today), Some(0));
    }

    #[test]
    fn test_determine_current_window_attempt_1() {
        let due_date = NaiveDate::from_ymd_opt(2026, 2, 15).unwrap();

        // On +3 days
        let today = NaiveDate::from_ymd_opt(2026, 2, 18).unwrap();
        assert_eq!(determine_current_window(due_date, today), Some(1));

        // Day after +3 days (still in attempt 1 window)
        let today = NaiveDate::from_ymd_opt(2026, 2, 19).unwrap();
        assert_eq!(determine_current_window(due_date, today), Some(1));
    }

    #[test]
    fn test_determine_current_window_attempt_2() {
        let due_date = NaiveDate::from_ymd_opt(2026, 2, 15).unwrap();

        // On +7 days
        let today = NaiveDate::from_ymd_opt(2026, 2, 22).unwrap();
        assert_eq!(determine_current_window(due_date, today), Some(2));

        // Day after +7 days (still in attempt 2 window)
        let today = NaiveDate::from_ymd_opt(2026, 2, 23).unwrap();
        assert_eq!(determine_current_window(due_date, today), Some(2));
    }

    #[test]
    fn test_determine_current_window_before_due() {
        let due_date = NaiveDate::from_ymd_opt(2026, 2, 15).unwrap();

        // Before due date
        let today = NaiveDate::from_ymd_opt(2026, 2, 14).unwrap();
        assert_eq!(determine_current_window(due_date, today), None);
    }

    #[test]
    fn test_is_eligible_for_retry() {
        assert!(is_eligible_for_retry("attempting"));
        assert!(is_eligible_for_retry("failed_retry"));

        assert!(!is_eligible_for_retry("succeeded"));
        assert!(!is_eligible_for_retry("failed_final"));
        assert!(!is_eligible_for_retry("unknown")); // UNKNOWN blocks retry
    }

    #[test]
    fn test_unknown_blocks_retry() {
        // CRITICAL TEST: Verify UNKNOWN status blocks retry eligibility
        assert!(
            !is_eligible_for_retry("unknown"),
            "UNKNOWN status must block retry scheduling (bd-2uw protocol)"
        );
    }

    #[test]
    fn test_retry_error_display() {
        let id = Uuid::nil();
        let err = RetryError::AttemptNotFound(id);
        assert_eq!(
            err.to_string(),
            format!("Payment attempt not found: {}", id)
        );

        let err = RetryError::NoDueDate {
            payment_id: id,
            invoice_id: "INV-123".to_string(),
        };
        assert_eq!(
            err.to_string(),
            format!("Payment {} (invoice INV-123) has no due date", id)
        );

        let err = RetryError::MaxAttemptsReached {
            payment_id: id,
            attempt_count: 3,
        };
        assert_eq!(
            err.to_string(),
            format!("Payment {} has reached max attempts (3)", id)
        );
    }

    #[test]
    fn test_window_constants_match_ar() {
        // Verify Payments retry windows match AR retry windows (bd-8ev)
        assert_eq!(windows::ATTEMPT_0_OFFSET_DAYS, 0);
        assert_eq!(windows::ATTEMPT_1_OFFSET_DAYS, 3);
        assert_eq!(windows::ATTEMPT_2_OFFSET_DAYS, 7);
        assert_eq!(windows::MAX_ATTEMPTS, 3);
    }
}
