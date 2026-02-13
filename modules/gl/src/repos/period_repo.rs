//! Repository for accounting period operations
//!
//! Provides database access for accounting periods to support closed-period governance.

use chrono::{DateTime, NaiveDate, Utc};
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use thiserror::Error;
use uuid::Uuid;

/// Accounting period model representing a fiscal/accounting period
#[derive(Debug, Clone, FromRow)]
pub struct AccountingPeriod {
    pub id: Uuid,
    pub tenant_id: String,
    pub period_start: NaiveDate,
    pub period_end: NaiveDate,
    pub is_closed: bool,
    pub created_at: DateTime<Utc>,
}

/// Errors that can occur during period repository operations
#[derive(Debug, Error)]
pub enum PeriodError {
    #[error("No accounting period found for tenant_id={tenant_id}, date={date}")]
    NoPeriodForDate { tenant_id: String, date: NaiveDate },

    #[error("Accounting period is closed: tenant_id={tenant_id}, date={date}, period_id={period_id}")]
    PeriodClosed {
        tenant_id: String,
        date: NaiveDate,
        period_id: Uuid,
    },

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

/// Find the accounting period that contains the given date for a tenant
/// Returns the period if found, None if no period contains the date
pub async fn find_by_date(
    pool: &PgPool,
    tenant_id: &str,
    date: NaiveDate,
) -> Result<Option<AccountingPeriod>, PeriodError> {
    let period = sqlx::query_as::<_, AccountingPeriod>(
        r#"
        SELECT id, tenant_id, period_start, period_end, is_closed, created_at
        FROM accounting_periods
        WHERE tenant_id = $1
          AND period_start <= $2
          AND period_end >= $2
        LIMIT 1
        "#,
    )
    .bind(tenant_id)
    .bind(date)
    .fetch_optional(pool)
    .await?;

    Ok(period)
}

/// Find the accounting period that contains the given date for a tenant within a transaction
/// Returns the period if found, None if no period contains the date
pub async fn find_by_date_tx(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    date: NaiveDate,
) -> Result<Option<AccountingPeriod>, PeriodError> {
    let period = sqlx::query_as::<_, AccountingPeriod>(
        r#"
        SELECT id, tenant_id, period_start, period_end, is_closed, created_at
        FROM accounting_periods
        WHERE tenant_id = $1
          AND period_start <= $2
          AND period_end >= $2
        LIMIT 1
        "#,
    )
    .bind(tenant_id)
    .bind(date)
    .fetch_optional(&mut **tx)
    .await?;

    Ok(period)
}

/// Validate that a date falls within an open accounting period
/// Returns error if no period exists for the date or if the period is closed
pub async fn validate_posting_date(
    pool: &PgPool,
    tenant_id: &str,
    date: NaiveDate,
) -> Result<(), PeriodError> {
    let period = find_by_date(pool, tenant_id, date).await?;

    match period {
        None => Err(PeriodError::NoPeriodForDate {
            tenant_id: tenant_id.to_string(),
            date,
        }),
        Some(p) if p.is_closed => Err(PeriodError::PeriodClosed {
            tenant_id: tenant_id.to_string(),
            date,
            period_id: p.id,
        }),
        Some(_) => Ok(()),
    }
}

/// Validate that a date falls within an open accounting period within a transaction
/// Returns error if no period exists for the date or if the period is closed
pub async fn validate_posting_date_tx(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    date: NaiveDate,
) -> Result<(), PeriodError> {
    let period = find_by_date_tx(tx, tenant_id, date).await?;

    match period {
        None => Err(PeriodError::NoPeriodForDate {
            tenant_id: tenant_id.to_string(),
            date,
        }),
        Some(p) if p.is_closed => Err(PeriodError::PeriodClosed {
            tenant_id: tenant_id.to_string(),
            date,
            period_id: p.id,
        }),
        Some(_) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_period_error_display() {
        let err = PeriodError::NoPeriodForDate {
            tenant_id: "tenant_123".to_string(),
            date: NaiveDate::from_ymd_opt(2024, 2, 11).unwrap(),
        };
        assert!(err.to_string().contains("tenant_123"));
        assert!(err.to_string().contains("2024-02-11"));
    }

    #[test]
    fn test_period_closed_error_display() {
        let err = PeriodError::PeriodClosed {
            tenant_id: "tenant_123".to_string(),
            date: NaiveDate::from_ymd_opt(2024, 2, 11).unwrap(),
            period_id: Uuid::new_v4(),
        };
        assert!(err.to_string().contains("closed"));
        assert!(err.to_string().contains("tenant_123"));
    }
}
