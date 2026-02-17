use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct FxRate {
    pub id: Uuid,
    pub tenant_id: String,
    pub base_currency: String,
    pub quote_currency: String,
    pub rate: f64,
    pub inverse_rate: f64,
    pub effective_at: DateTime<Utc>,
    pub source: String,
    pub idempotency_key: String,
    pub created_at: DateTime<Utc>,
}

/// Insert an FX rate within a transaction. Returns true if inserted, false if
/// the idempotency_key already exists (no-op).
pub async fn insert_fx_rate(
    tx: &mut Transaction<'_, Postgres>,
    rate: &FxRate,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        r#"
        INSERT INTO fx_rates (
            id, tenant_id, base_currency, quote_currency,
            rate, inverse_rate, effective_at, source,
            idempotency_key, created_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        ON CONFLICT (idempotency_key) DO NOTHING
        "#,
    )
    .bind(rate.id)
    .bind(&rate.tenant_id)
    .bind(&rate.base_currency)
    .bind(&rate.quote_currency)
    .bind(rate.rate)
    .bind(rate.inverse_rate)
    .bind(rate.effective_at)
    .bind(&rate.source)
    .bind(&rate.idempotency_key)
    .bind(rate.created_at)
    .execute(&mut **tx)
    .await?;

    Ok(result.rows_affected() > 0)
}

/// Get the latest FX rate for a currency pair as-of a given timestamp.
///
/// Returns the rate with the highest effective_at that is <= `as_of`.
/// Deterministic: for ties, picks the one inserted first (by created_at, then id).
pub async fn get_latest_rate(
    pool: &PgPool,
    tenant_id: &str,
    base_currency: &str,
    quote_currency: &str,
    as_of: DateTime<Utc>,
) -> Result<Option<FxRate>, sqlx::Error> {
    let row = sqlx::query(
        r#"
        SELECT id, tenant_id, base_currency, quote_currency,
               rate, inverse_rate, effective_at, source,
               idempotency_key, created_at
        FROM fx_rates
        WHERE tenant_id = $1
          AND base_currency = $2
          AND quote_currency = $3
          AND effective_at <= $4
        ORDER BY effective_at DESC, created_at ASC, id ASC
        LIMIT 1
        "#,
    )
    .bind(tenant_id)
    .bind(base_currency)
    .bind(quote_currency)
    .bind(as_of)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| FxRate {
        id: r.get("id"),
        tenant_id: r.get("tenant_id"),
        base_currency: r.get("base_currency"),
        quote_currency: r.get("quote_currency"),
        rate: r.get("rate"),
        inverse_rate: r.get("inverse_rate"),
        effective_at: r.get("effective_at"),
        source: r.get("source"),
        idempotency_key: r.get("idempotency_key"),
        created_at: r.get("created_at"),
    }))
}

/// List all rates for a currency pair within a tenant, ordered by effective_at DESC.
pub async fn list_rates(
    pool: &PgPool,
    tenant_id: &str,
    base_currency: &str,
    quote_currency: &str,
) -> Result<Vec<FxRate>, sqlx::Error> {
    let rows = sqlx::query(
        r#"
        SELECT id, tenant_id, base_currency, quote_currency,
               rate, inverse_rate, effective_at, source,
               idempotency_key, created_at
        FROM fx_rates
        WHERE tenant_id = $1
          AND base_currency = $2
          AND quote_currency = $3
        ORDER BY effective_at DESC, created_at ASC, id ASC
        "#,
    )
    .bind(tenant_id)
    .bind(base_currency)
    .bind(quote_currency)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| FxRate {
            id: r.get("id"),
            tenant_id: r.get("tenant_id"),
            base_currency: r.get("base_currency"),
            quote_currency: r.get("quote_currency"),
            rate: r.get("rate"),
            inverse_rate: r.get("inverse_rate"),
            effective_at: r.get("effective_at"),
            source: r.get("source"),
            idempotency_key: r.get("idempotency_key"),
            created_at: r.get("created_at"),
        })
        .collect())
}
