//! Repository for report query operations (Phase 12)
//!
//! Provides read-only, bounded queries for reporting primitives.
//! All queries are tenant-scoped and designed to use indexes.
//!
//! **Performance Contract**: All queries must execute in < 500ms at normal scale (100K entries/tenant)

mod account_activity;
mod journal_entries;
mod period_entries;

pub use account_activity::*;
pub use journal_entries::*;
pub use period_entries::*;

use chrono::{DateTime, Utc};
use thiserror::Error;

/// Errors that can occur during report query operations
#[derive(Debug, Error)]
pub enum ReportQueryError {
    #[error("Account not found: tenant_id={tenant_id}, code={code}")]
    AccountNotFound { tenant_id: String, code: String },

    #[error("Invalid date range: start {start} is after end {end}")]
    InvalidDateRange {
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    },

    #[error("Invalid pagination parameters: limit={limit}, offset={offset}")]
    InvalidPagination { limit: i64, offset: i64 },

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_report_query_error_display() {
        let err = ReportQueryError::AccountNotFound {
            tenant_id: "tenant1".to_string(),
            code: "1000".to_string(),
        };
        assert!(err.to_string().contains("tenant1"));
        assert!(err.to_string().contains("1000"));

        let start = Utc::now();
        let end = start - chrono::Duration::hours(1);
        let err = ReportQueryError::InvalidDateRange { start, end };
        assert!(err.to_string().contains("is after"));
    }
}
