//! Period journal entry queries — header-only listings for period reporting

use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use super::ReportQueryError;

/// Period journal entry (header-only for listing)
#[derive(Debug, Clone, FromRow)]
pub struct PeriodJournalEntry {
    pub id: Uuid,
    pub posted_at: DateTime<Utc>,
    pub description: Option<String>,
    pub currency: String,
    pub source_module: String,
}

/// Query journal entries for a period (header-only listing)
///
/// Returns journal entry headers ordered by posted_at DESC.
/// Use this for period journal listings without fetching all lines.
pub async fn query_period_journal_entries(
    pool: &PgPool,
    tenant_id: &str,
    start_date: DateTime<Utc>,
    end_date: DateTime<Utc>,
    limit: i64,
    offset: i64,
) -> Result<Vec<PeriodJournalEntry>, ReportQueryError> {
    // Validate date range
    if start_date > end_date {
        return Err(ReportQueryError::InvalidDateRange {
            start: start_date,
            end: end_date,
        });
    }

    // Validate pagination
    if limit <= 0 || offset < 0 {
        return Err(ReportQueryError::InvalidPagination { limit, offset });
    }

    let entries = sqlx::query_as::<_, PeriodJournalEntry>(
        r#"
        SELECT id, posted_at, description, currency, source_module
        FROM journal_entries
        WHERE tenant_id = $1
          AND posted_at >= $2
          AND posted_at <= $3
        ORDER BY posted_at DESC, created_at DESC
        LIMIT $4 OFFSET $5
        "#,
    )
    .bind(tenant_id)
    .bind(start_date)
    .bind(end_date)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    Ok(entries)
}
