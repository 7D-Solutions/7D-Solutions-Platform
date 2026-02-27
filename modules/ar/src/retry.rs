//! AR Retry Window Discipline Module (Phase 15 - bd-8ev)
//!
//! **Fixed Retry Windows:** This module implements deterministic retry scheduling
//! for invoice payment collection attempts.
//!
//! **Retry Schedule:**
//! - Attempt 0: Due date (immediate)
//! - Attempt 1: Due date + 3 days
//! - Attempt 2: Due date + 7 days
//!
//! **Critical Invariant (ChatGPT):**
//! Exactly one attempt per window, enforced by attempt_no derived from window index.
//! No configurability - windows are hard-coded for deterministic behavior.
//!
//! **Integration:**
//! - Uses ar_invoice_attempts UNIQUE(app_id, invoice_id, attempt_no) from bd-7gl
//! - Calls ar::finalization::finalize_invoice from bd-3fo
//! - Respects ar::lifecycle guards from bd-1w7

use chrono::{NaiveDate, Utc};
use sqlx::PgPool;
use std::fmt;

// ============================================================================
// Retry Window Configuration
// ============================================================================

/// Fixed retry windows (no configurability)
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
    /// Invoice not found
    InvoiceNotFound(i32),
    /// Invoice has no due date set
    NoDueDate(i32),
    /// Invoice not eligible for retry (wrong status)
    NotEligible {
        invoice_id: i32,
        current_status: String,
        reason: String,
    },
    /// No more retry windows available (max attempts reached)
    MaxAttemptsReached { invoice_id: i32, attempt_count: i32 },
    /// Database error
    DatabaseError(String),
}

impl fmt::Display for RetryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvoiceNotFound(id) => write!(f, "Invoice not found: {}", id),
            Self::NoDueDate(id) => write!(f, "Invoice {} has no due date", id),
            Self::NotEligible {
                invoice_id,
                current_status,
                reason,
            } => write!(
                f,
                "Invoice {} not eligible for retry (status: {}): {}",
                invoice_id, current_status, reason
            ),
            Self::MaxAttemptsReached {
                invoice_id,
                attempt_count,
            } => write!(
                f,
                "Invoice {} has reached max attempts ({})",
                invoice_id, attempt_count
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

/// Calculate retry window dates for an invoice
///
/// **Returns:** Array of 3 dates: [attempt_0_date, attempt_1_date, attempt_2_date]
///
/// **Example:**
/// ```rust,no_run
/// use chrono::NaiveDate;
/// use ar_rs::retry::calculate_retry_windows;
///
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
/// ```rust,no_run
/// use chrono::NaiveDate;
/// use ar_rs::retry::determine_current_window;
///
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

/// Check if invoice is eligible for retry based on current status
///
/// **Eligible statuses:**
/// - "open" (first attempt)
/// - "attempting" (retry attempts)
/// - "failed_retry" (retry attempts)
///
/// **Ineligible statuses:**
/// - "paid" (terminal - success)
/// - "failed_final" (terminal - max retries exhausted)
/// - "void" (terminal - cancelled)
pub fn is_eligible_for_retry(status: &str) -> bool {
    matches!(status, "open" | "attempting" | "failed_retry")
}

// ============================================================================
// Retry Scheduling
// ============================================================================

/// Get invoices eligible for retry in current window
///
/// **Returns:** List of invoice IDs that should be retried today
///
/// **Logic:**
/// 1. Fetch invoices with eligible status (open, attempting, failed_retry)
/// 2. Calculate which retry window they're in based on due_date
/// 3. Check if attempt already exists for that window
/// 4. Return invoices that need attempt in current window
pub async fn get_invoices_for_retry(
    pool: &PgPool,
    app_id: &str,
) -> Result<Vec<(i32, i32)>, RetryError> {
    // Fetch invoices eligible for retry
    let invoices: Vec<(i32, String, Option<NaiveDate>)> = sqlx::query_as(
        "SELECT id, status, due_at::date
         FROM ar_invoices
         WHERE app_id = $1
           AND status IN ('open', 'attempting', 'failed_retry')
           AND due_at IS NOT NULL
         ORDER BY due_at ASC",
    )
    .bind(app_id)
    .fetch_all(pool)
    .await?;

    let today = Utc::now().date_naive();
    let mut retry_list = Vec::new();

    for (invoice_id, _status, due_date_opt) in invoices {
        let due_date = match due_date_opt {
            Some(d) => d,
            None => continue, // Skip invoices without due date
        };

        // Determine current retry window
        let current_window = match determine_current_window(due_date, today) {
            Some(w) => w,
            None => continue, // Not in any window yet
        };

        // Check if attempt already exists for this window
        let attempt_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(
                SELECT 1 FROM ar_invoice_attempts
                WHERE app_id = $1 AND invoice_id = $2 AND attempt_no = $3
            )",
        )
        .bind(app_id)
        .bind(invoice_id)
        .bind(current_window)
        .fetch_one(pool)
        .await?;

        if !attempt_exists {
            retry_list.push((invoice_id, current_window));
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
        assert!(is_eligible_for_retry("open"));
        assert!(is_eligible_for_retry("attempting"));
        assert!(is_eligible_for_retry("failed_retry"));

        assert!(!is_eligible_for_retry("paid"));
        assert!(!is_eligible_for_retry("failed_final"));
        assert!(!is_eligible_for_retry("void"));
    }

    #[test]
    fn test_retry_error_display() {
        let err = RetryError::InvoiceNotFound(123);
        assert_eq!(err.to_string(), "Invoice not found: 123");

        let err = RetryError::NoDueDate(456);
        assert_eq!(err.to_string(), "Invoice 456 has no due date");

        let err = RetryError::MaxAttemptsReached {
            invoice_id: 789,
            attempt_count: 3,
        };
        assert_eq!(err.to_string(), "Invoice 789 has reached max attempts (3)");
    }
}
