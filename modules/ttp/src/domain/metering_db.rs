/// Database query helpers for TTP metering.
///
/// Extracted from metering.rs to stay under the 500 LOC file size limit.
use chrono::NaiveDate;
use sqlx::{PgPool, Row};
use std::collections::HashMap;
use uuid::Uuid;

use super::metering::{DimensionAggregate, IngestResult, MeteringError, MeteringEventInput};

// ---------------------------------------------------------------------------
// Ingestion
// ---------------------------------------------------------------------------

/// Insert a single metering event. ON CONFLICT DO NOTHING for idempotency.
pub async fn insert_event(
    pool: &PgPool,
    input: &MeteringEventInput,
) -> Result<IngestResult, MeteringError> {
    let event_id = Uuid::new_v4();

    let result = sqlx::query(
        r#"
        INSERT INTO ttp_metering_events
            (event_id, tenant_id, dimension, quantity, occurred_at, idempotency_key, source_ref)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        ON CONFLICT (tenant_id, idempotency_key) DO NOTHING
        "#,
    )
    .bind(event_id)
    .bind(input.tenant_id)
    .bind(&input.dimension)
    .bind(input.quantity)
    .bind(input.occurred_at)
    .bind(&input.idempotency_key)
    .bind(&input.source_ref)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        Ok(IngestResult {
            event_id: None,
            was_duplicate: true,
        })
    } else {
        Ok(IngestResult {
            event_id: Some(event_id),
            was_duplicate: false,
        })
    }
}

// ---------------------------------------------------------------------------
// Aggregation
// ---------------------------------------------------------------------------

/// Aggregate metering events by dimension for a time range.
///
/// Returns aggregates sorted by dimension for deterministic output.
/// Time range: [period_start, period_end) — half-open interval.
pub async fn aggregate_by_dimension(
    pool: &PgPool,
    tenant_id: Uuid,
    period_start: NaiveDate,
    period_end: NaiveDate,
) -> Result<Vec<DimensionAggregate>, MeteringError> {
    let rows = sqlx::query(
        r#"
        SELECT dimension,
               CAST(SUM(quantity) AS BIGINT) AS total_quantity,
               COUNT(*)                      AS event_count
        FROM ttp_metering_events
        WHERE tenant_id = $1
          AND occurred_at >= $2::date::timestamptz
          AND occurred_at <  $3::date::timestamptz
        GROUP BY dimension
        ORDER BY dimension
        "#,
    )
    .bind(tenant_id)
    .bind(period_start)
    .bind(period_end)
    .fetch_all(pool)
    .await?;

    let mut aggregates = Vec::with_capacity(rows.len());
    for row in &rows {
        aggregates.push(DimensionAggregate {
            dimension: row.try_get("dimension")?,
            total_quantity: row.try_get("total_quantity")?,
            event_count: row.try_get("event_count")?,
        });
    }
    Ok(aggregates)
}

// ---------------------------------------------------------------------------
// Pricing lookup
// ---------------------------------------------------------------------------

/// Get effective pricing rules for a tenant at a given date.
///
/// Returns a map: dimension → (unit_price_minor, currency).
/// For each dimension, selects the rule with the latest effective_from ≤ date
/// and (effective_to IS NULL OR effective_to > date).
pub async fn get_pricing_rules(
    pool: &PgPool,
    tenant_id: Uuid,
    effective_date: NaiveDate,
) -> Result<HashMap<String, (i64, String)>, MeteringError> {
    let rows = sqlx::query(
        r#"
        SELECT DISTINCT ON (dimension)
               dimension, unit_price_minor, currency
        FROM ttp_metering_pricing
        WHERE tenant_id = $1
          AND effective_from <= $2
          AND (effective_to IS NULL OR effective_to > $2)
        ORDER BY dimension, effective_from DESC
        "#,
    )
    .bind(tenant_id)
    .bind(effective_date)
    .fetch_all(pool)
    .await?;

    let mut map = HashMap::new();
    for row in &rows {
        let dim: String = row.try_get("dimension")?;
        let price: i64 = row.try_get("unit_price_minor")?;
        let currency: String = row.try_get("currency")?;
        map.insert(dim, (price, currency));
    }
    Ok(map)
}
