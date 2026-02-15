//! Subscription Cycle Gating Module
//!
//! Phase 15 bd-184: Exactly-once invoice generation per subscription cycle.
//!
//! # Guarantees
//! 1. **Exactly-once:** No duplicate invoices for same subscription cycle
//! 2. **Concurrent-safe:** pg_advisory_xact_lock prevents races
//! 3. **Replay-safe:** UNIQUE constraint on (tenant_id, subscription_id, cycle_key)
//! 4. **Deterministic:** cycle_key derived from stable cycle boundaries
//!
//! # Pattern
//! ```
//! Gate → Lock → Check → Execute → Record
//! ```
//!
//! # Integration
//! - Uses bd-1p2 idempotency key spec (cycle boundaries)
//! - Uses bd-7gl attempt ledger pattern (UNIQUE constraints)
//! - Wraps invoice creation in gated transaction

use chrono::{Datelike, NaiveDate};
use sqlx::{PgConnection, PgPool};
use thiserror::Error;
use uuid::Uuid;

// ============================================================================
// Error Types
// ============================================================================

#[derive(Debug, Error)]
pub enum CycleGatingError {
    #[error("Duplicate invoice attempt for subscription {subscription_id} cycle {cycle_key}")]
    DuplicateCycle {
        subscription_id: Uuid,
        cycle_key: String,
    },

    #[error("Invalid cycle key format: {cycle_key}")]
    InvalidCycleKey { cycle_key: String },

    #[error("Database error: {source}")]
    DatabaseError {
        #[from]
        source: sqlx::Error,
    },
}

// ============================================================================
// Cycle Key Generation
// ============================================================================

/// Generate deterministic cycle key from date.
///
/// **Format:** YYYY-MM (e.g., "2026-02")
///
/// **Stability:** Monthly billing cycles always resolve to same key
/// regardless of when execute_bill_run is triggered.
///
/// **Example:**
/// ```
/// let date = NaiveDate::from_ymd_opt(2026, 2, 15).unwrap();
/// let key = generate_cycle_key(date);
/// assert_eq!(key, "2026-02");
/// ```
pub fn generate_cycle_key(date: NaiveDate) -> String {
    format!("{:04}-{:02}", date.year(), date.month())
}

/// Calculate cycle boundaries from a date.
///
/// **Returns:** (cycle_start, cycle_end) as first and last day of month
///
/// **Example:**
/// ```
/// let date = NaiveDate::from_ymd_opt(2026, 2, 15).unwrap();
/// let (start, end) = calculate_cycle_boundaries(date);
/// assert_eq!(start, NaiveDate::from_ymd_opt(2026, 2, 1).unwrap());
/// assert_eq!(end, NaiveDate::from_ymd_opt(2026, 2, 28).unwrap());
/// ```
pub fn calculate_cycle_boundaries(date: NaiveDate) -> (NaiveDate, NaiveDate) {
    let year = date.year();
    let month = date.month();

    // First day of month
    let cycle_start = NaiveDate::from_ymd_opt(year, month, 1)
        .expect("Invalid cycle start date");

    // Last day of month
    let cycle_end = if month == 12 {
        NaiveDate::from_ymd_opt(year, 12, 31).expect("Invalid cycle end date")
    } else {
        // Last day = (first day of next month) - 1 day
        NaiveDate::from_ymd_opt(year, month + 1, 1)
            .expect("Invalid next month date")
            .pred_opt()
            .expect("Invalid cycle end date")
    };

    (cycle_start, cycle_end)
}

// ============================================================================
// Advisory Lock Helpers
// ============================================================================

/// Generate deterministic advisory lock key from subscription cycle.
///
/// **Pattern:** Hash (tenant_id, subscription_id, cycle_key) → i64
///
/// **Scope:** Transaction-scoped (pg_advisory_xact_lock)
///
/// **Collision:** Minimal (64-bit hash space)
fn generate_advisory_lock_key(
    tenant_id: &str,
    subscription_id: Uuid,
    cycle_key: &str,
) -> i64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    tenant_id.hash(&mut hasher);
    subscription_id.hash(&mut hasher);
    cycle_key.hash(&mut hasher);

    let hash = hasher.finish();
    // Convert u64 to i64 (Postgres advisory lock takes i64)
    hash as i64
}

/// Acquire transaction-scoped advisory lock.
///
/// **Critical:** Lock is automatically released on transaction commit/rollback
///
/// **Usage:**
/// ```rust
/// let mut tx = pool.begin().await?;
/// acquire_cycle_lock(&mut tx, tenant_id, subscription_id, cycle_key).await?;
/// // ... perform gated operation ...
/// tx.commit().await?;  // Lock automatically released
/// ```
pub async fn acquire_cycle_lock(
    tx: &mut PgConnection,
    tenant_id: &str,
    subscription_id: Uuid,
    cycle_key: &str,
) -> Result<(), CycleGatingError> {
    let lock_key = generate_advisory_lock_key(tenant_id, subscription_id, cycle_key);

    tracing::debug!(
        tenant_id = tenant_id,
        subscription_id = %subscription_id,
        cycle_key = cycle_key,
        lock_key = lock_key,
        "Acquiring advisory lock for subscription cycle"
    );

    // pg_advisory_xact_lock: Transaction-scoped, auto-released on commit/rollback
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(lock_key)
        .execute(&mut *tx)
        .await?;

    tracing::debug!(
        tenant_id = tenant_id,
        subscription_id = %subscription_id,
        cycle_key = cycle_key,
        "Advisory lock acquired"
    );

    Ok(())
}

// ============================================================================
// Attempt Ledger Operations
// ============================================================================

/// Check if invoice generation attempt already exists for this cycle.
///
/// **Returns:** true if attempt exists (duplicate), false if new
pub async fn cycle_attempt_exists(
    tx: &mut PgConnection,
    tenant_id: &str,
    subscription_id: Uuid,
    cycle_key: &str,
) -> Result<bool, CycleGatingError> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM subscription_invoice_attempts
         WHERE tenant_id = $1 AND subscription_id = $2 AND cycle_key = $3"
    )
    .bind(tenant_id)
    .bind(subscription_id)
    .bind(cycle_key)
    .fetch_one(&mut *tx)
    .await?;

    Ok(count > 0)
}

/// Record invoice generation attempt in ledger.
///
/// **Critical:** UNIQUE constraint (tenant_id, subscription_id, cycle_key)
/// provides database-level duplicate prevention.
///
/// **Status:** 'attempting' initially, updated to 'succeeded' or 'failed_*' later
pub async fn record_cycle_attempt(
    tx: &mut PgConnection,
    tenant_id: &str,
    subscription_id: Uuid,
    cycle_key: &str,
    cycle_start: NaiveDate,
    cycle_end: NaiveDate,
    idempotency_key: Option<&str>,
) -> Result<Uuid, CycleGatingError> {
    let attempt_id: Uuid = sqlx::query_scalar(
        "INSERT INTO subscription_invoice_attempts (
            tenant_id, subscription_id, cycle_key, cycle_start, cycle_end,
            status, idempotency_key
         )
         VALUES ($1, $2, $3, $4, $5, 'attempting', $6)
         RETURNING id"
    )
    .bind(tenant_id)
    .bind(subscription_id)
    .bind(cycle_key)
    .bind(cycle_start)
    .bind(cycle_end)
    .bind(idempotency_key)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| {
        // Check if error is UNIQUE constraint violation
        if let Some(db_err) = e.as_database_error() {
            if db_err.constraint() == Some("unique_subscription_cycle_invoice") {
                return CycleGatingError::DuplicateCycle {
                    subscription_id,
                    cycle_key: cycle_key.to_string(),
                };
            }
        }
        CycleGatingError::DatabaseError { source: e }
    })?;

    tracing::info!(
        tenant_id = tenant_id,
        subscription_id = %subscription_id,
        cycle_key = cycle_key,
        attempt_id = %attempt_id,
        "Recorded cycle attempt"
    );

    Ok(attempt_id)
}

/// Update attempt status to 'succeeded' with AR invoice ID.
pub async fn mark_attempt_succeeded(
    tx: &mut PgConnection,
    attempt_id: Uuid,
    ar_invoice_id: i32,
) -> Result<(), CycleGatingError> {
    sqlx::query(
        "UPDATE subscription_invoice_attempts
         SET status = 'succeeded', ar_invoice_id = $2, completed_at = NOW(), updated_at = NOW()
         WHERE id = $1"
    )
    .bind(attempt_id)
    .bind(ar_invoice_id)
    .execute(&mut *tx)
    .await?;

    tracing::info!(
        attempt_id = %attempt_id,
        ar_invoice_id = ar_invoice_id,
        "Marked attempt as succeeded"
    );

    Ok(())
}

/// Update attempt status to 'failed_final' with error details.
pub async fn mark_attempt_failed(
    tx: &mut PgConnection,
    attempt_id: Uuid,
    failure_code: &str,
    failure_message: &str,
) -> Result<(), CycleGatingError> {
    sqlx::query(
        "UPDATE subscription_invoice_attempts
         SET status = 'failed_final', failure_code = $2, failure_message = $3,
             completed_at = NOW(), updated_at = NOW()
         WHERE id = $1"
    )
    .bind(attempt_id)
    .bind(failure_code)
    .bind(failure_message)
    .execute(&mut *tx)
    .await?;

    tracing::warn!(
        attempt_id = %attempt_id,
        failure_code = failure_code,
        failure_message = failure_message,
        "Marked attempt as failed"
    );

    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_cycle_key() {
        let date = NaiveDate::from_ymd_opt(2026, 2, 15).unwrap();
        let key = generate_cycle_key(date);
        assert_eq!(key, "2026-02");
    }

    #[test]
    fn test_generate_cycle_key_january() {
        let date = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let key = generate_cycle_key(date);
        assert_eq!(key, "2026-01");
    }

    #[test]
    fn test_generate_cycle_key_december() {
        let date = NaiveDate::from_ymd_opt(2026, 12, 31).unwrap();
        let key = generate_cycle_key(date);
        assert_eq!(key, "2026-12");
    }

    #[test]
    fn test_cycle_key_determinism() {
        let date1 = NaiveDate::from_ymd_opt(2026, 2, 1).unwrap();
        let date2 = NaiveDate::from_ymd_opt(2026, 2, 15).unwrap();
        let date3 = NaiveDate::from_ymd_opt(2026, 2, 28).unwrap();

        assert_eq!(generate_cycle_key(date1), generate_cycle_key(date2));
        assert_eq!(generate_cycle_key(date2), generate_cycle_key(date3));
    }

    #[test]
    fn test_calculate_cycle_boundaries_february() {
        let date = NaiveDate::from_ymd_opt(2026, 2, 15).unwrap();
        let (start, end) = calculate_cycle_boundaries(date);

        assert_eq!(start, NaiveDate::from_ymd_opt(2026, 2, 1).unwrap());
        assert_eq!(end, NaiveDate::from_ymd_opt(2026, 2, 28).unwrap());
    }

    #[test]
    fn test_calculate_cycle_boundaries_december() {
        let date = NaiveDate::from_ymd_opt(2026, 12, 15).unwrap();
        let (start, end) = calculate_cycle_boundaries(date);

        assert_eq!(start, NaiveDate::from_ymd_opt(2026, 12, 1).unwrap());
        assert_eq!(end, NaiveDate::from_ymd_opt(2026, 12, 31).unwrap());
    }

    #[test]
    fn test_calculate_cycle_boundaries_january() {
        let date = NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();
        let (start, end) = calculate_cycle_boundaries(date);

        assert_eq!(start, NaiveDate::from_ymd_opt(2026, 1, 1).unwrap());
        assert_eq!(end, NaiveDate::from_ymd_opt(2026, 1, 31).unwrap());
    }

    #[test]
    fn test_calculate_cycle_boundaries_leap_year() {
        // 2024 is a leap year
        let date = NaiveDate::from_ymd_opt(2024, 2, 15).unwrap();
        let (start, end) = calculate_cycle_boundaries(date);

        assert_eq!(start, NaiveDate::from_ymd_opt(2024, 2, 1).unwrap());
        assert_eq!(end, NaiveDate::from_ymd_opt(2024, 2, 29).unwrap());
    }

    #[test]
    fn test_advisory_lock_key_determinism() {
        let tenant_id = "tenant-123";
        let subscription_id = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let cycle_key = "2026-02";

        let key1 = generate_advisory_lock_key(tenant_id, subscription_id, cycle_key);
        let key2 = generate_advisory_lock_key(tenant_id, subscription_id, cycle_key);

        assert_eq!(key1, key2);
    }

    #[test]
    fn test_advisory_lock_key_uniqueness() {
        let tenant_id = "tenant-123";
        let sub_id1 = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let sub_id2 = Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
        let cycle_key = "2026-02";

        let key1 = generate_advisory_lock_key(tenant_id, sub_id1, cycle_key);
        let key2 = generate_advisory_lock_key(tenant_id, sub_id2, cycle_key);

        assert_ne!(key1, key2);
    }

    #[test]
    fn test_advisory_lock_key_cycle_uniqueness() {
        let tenant_id = "tenant-123";
        let subscription_id = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();

        let key1 = generate_advisory_lock_key(tenant_id, subscription_id, "2026-02");
        let key2 = generate_advisory_lock_key(tenant_id, subscription_id, "2026-03");

        assert_ne!(key1, key2);
    }
}
