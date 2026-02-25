/// TTP metering domain logic: per-event usage tracking.
///
/// # Idempotency model
///
/// Each metering event carries (tenant_id, idempotency_key). The UNIQUE
/// constraint on this pair means duplicate submissions are silently ignored
/// (ON CONFLICT DO NOTHING). This guarantees exactly-once semantics at the
/// ingestion boundary.
///
/// # Deterministic aggregation
///
/// Aggregation always:
///   1. Filters events by tenant_id + time range [period_start, period_end).
///   2. Orders by (occurred_at, event_id) for stable iteration.
///   3. Groups by dimension, summing quantity.
///   4. Applies pricing rules effective at the period's start date.
///   5. Computes line_total = quantity * unit_price_minor (integer arithmetic, no rounding).
///
/// Same inputs → same outputs, always.
use chrono::{DateTime, NaiveDate, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use super::metering_db;

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum MeteringError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),

    #[error("invalid billing period: {0}")]
    InvalidPeriod(String),
}

// ---------------------------------------------------------------------------
// Value objects
// ---------------------------------------------------------------------------

/// A single metering event to ingest.
#[derive(Debug, Clone)]
pub struct MeteringEventInput {
    pub tenant_id: Uuid,
    pub dimension: String,
    pub quantity: i64,
    pub occurred_at: DateTime<Utc>,
    pub idempotency_key: String,
    pub source_ref: Option<String>,
}

/// Result of ingesting a single event.
#[derive(Debug)]
pub struct IngestResult {
    pub event_id: Option<Uuid>,
    /// `true` if the event already existed (idempotent no-op).
    pub was_duplicate: bool,
}

/// Aggregated usage for one dimension in a billing period.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DimensionAggregate {
    pub dimension: String,
    pub total_quantity: i64,
    pub event_count: i64,
}

/// A single line in the price trace.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TraceLineItem {
    pub dimension: String,
    pub total_quantity: i64,
    pub event_count: i64,
    pub unit_price_minor: i64,
    pub currency: String,
    pub line_total_minor: i64,
}

/// Full price trace for a billing period.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PriceTrace {
    pub tenant_id: Uuid,
    pub period: String,
    pub period_start: NaiveDate,
    pub period_end: NaiveDate,
    pub line_items: Vec<TraceLineItem>,
    pub total_minor: i64,
    pub currency: String,
}

// ---------------------------------------------------------------------------
// Period parsing
// ---------------------------------------------------------------------------

/// Parse "YYYY-MM" into (first day of month, first day of next month).
pub fn parse_period(period: &str) -> Result<(NaiveDate, NaiveDate), MeteringError> {
    let parts: Vec<&str> = period.splitn(2, '-').collect();
    if parts.len() != 2 {
        return Err(MeteringError::InvalidPeriod(period.to_string()));
    }
    let year: i32 = parts[0]
        .parse()
        .map_err(|_| MeteringError::InvalidPeriod(period.to_string()))?;
    let month: u32 = parts[1]
        .parse()
        .map_err(|_| MeteringError::InvalidPeriod(period.to_string()))?;

    if !(1..=12).contains(&month) {
        return Err(MeteringError::InvalidPeriod(period.to_string()));
    }

    let start = NaiveDate::from_ymd_opt(year, month, 1)
        .ok_or_else(|| MeteringError::InvalidPeriod(period.to_string()))?;

    let end = if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1)
    }
    .ok_or_else(|| MeteringError::InvalidPeriod(period.to_string()))?;

    Ok((start, end))
}

// ---------------------------------------------------------------------------
// Core operations
// ---------------------------------------------------------------------------

/// Ingest a single metering event. Idempotent: duplicate idempotency_key
/// for the same tenant is a no-op.
pub async fn ingest_event(
    pool: &PgPool,
    input: &MeteringEventInput,
) -> Result<IngestResult, MeteringError> {
    metering_db::insert_event(pool, input).await
}

/// Ingest a batch of metering events. Each is individually idempotent.
pub async fn ingest_events(
    pool: &PgPool,
    inputs: &[MeteringEventInput],
) -> Result<Vec<IngestResult>, MeteringError> {
    let mut results = Vec::with_capacity(inputs.len());
    for input in inputs {
        results.push(metering_db::insert_event(pool, input).await?);
    }
    Ok(results)
}

/// Compute the full price trace for a tenant + billing period.
///
/// Deterministic: same events + same pricing rules = same output, always.
pub async fn compute_price_trace(
    pool: &PgPool,
    tenant_id: Uuid,
    period: &str,
) -> Result<PriceTrace, MeteringError> {
    let (period_start, period_end) = parse_period(period)?;

    // 1. Aggregate events by dimension (stable ordering guaranteed by DB query)
    let aggregates =
        metering_db::aggregate_by_dimension(pool, tenant_id, period_start, period_end).await?;

    // 2. Look up pricing rules effective at the period start date
    let pricing =
        metering_db::get_pricing_rules(pool, tenant_id, period_start).await?;

    // 3. Apply pricing to aggregates (integer arithmetic, no rounding)
    let mut line_items = Vec::new();
    let mut total_minor: i64 = 0;
    let mut currency = "usd".to_string();

    for agg in &aggregates {
        let (unit_price, cur) = pricing
            .get(&agg.dimension)
            .map(|(p, c)| (*p, c.clone()))
            .unwrap_or((0, "usd".to_string()));

        let line_total = agg.total_quantity * unit_price;
        total_minor += line_total;
        currency = cur.clone();

        line_items.push(TraceLineItem {
            dimension: agg.dimension.clone(),
            total_quantity: agg.total_quantity,
            event_count: agg.event_count,
            unit_price_minor: unit_price,
            currency: cur,
            line_total_minor: line_total,
        });
    }

    // Sort line items by dimension for deterministic output ordering
    line_items.sort_by(|a, b| a.dimension.cmp(&b.dimension));

    Ok(PriceTrace {
        tenant_id,
        period: period.to_string(),
        period_start,
        period_end,
        line_items,
        total_minor,
        currency,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_period_valid() {
        let (start, end) = parse_period("2026-02").unwrap();
        assert_eq!(start, NaiveDate::from_ymd_opt(2026, 2, 1).unwrap());
        assert_eq!(end, NaiveDate::from_ymd_opt(2026, 3, 1).unwrap());
    }

    #[test]
    fn parse_period_december_wraps_year() {
        let (start, end) = parse_period("2026-12").unwrap();
        assert_eq!(start, NaiveDate::from_ymd_opt(2026, 12, 1).unwrap());
        assert_eq!(end, NaiveDate::from_ymd_opt(2027, 1, 1).unwrap());
    }

    #[test]
    fn parse_period_invalid_month() {
        assert!(parse_period("2026-13").is_err());
        assert!(parse_period("2026-00").is_err());
    }

    #[test]
    fn parse_period_invalid_format() {
        assert!(parse_period("202602").is_err());
        assert!(parse_period("").is_err());
        assert!(parse_period("2026").is_err());
    }
}
