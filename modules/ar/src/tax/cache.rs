//! Tax quote caching layer (bd-29j) — cache-through semantics for tax quotes.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use tax_core::models::*;
use tax_core::{TaxProvider, TaxProviderError};
use super::providers::LocalTaxProvider;

// ============================================================================
// Cached quote row
// ============================================================================

/// Cached tax quote row from ar_tax_quote_cache
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedTaxQuote {
    pub id: Uuid,
    pub app_id: String,
    pub invoice_id: String,
    pub idempotency_key: String,
    pub request_hash: String,
    pub provider: String,
    pub provider_quote_ref: String,
    pub total_tax_minor: i64,
    pub tax_by_line: serde_json::Value,
    pub response_json: serde_json::Value,
    pub quoted_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
}

// ============================================================================
// Request hash
// ============================================================================

/// Compute a deterministic SHA-256 hash of a tax quote request.
///
/// The hash is derived from the canonical JSON of (line_items, ship_to, ship_from, currency).
/// This ensures that if the invoice's taxable content hasn't changed, the same hash is produced,
/// allowing cache hits even across process restarts.
pub fn compute_request_hash(req: &TaxQuoteRequest) -> String {
    use sha2::{Digest, Sha256};

    // Canonicalize: sort-stable JSON of the fields that affect tax calculation
    let canonical = serde_json::json!({
        "line_items": req.line_items,
        "ship_to": req.ship_to,
        "ship_from": req.ship_from,
        "currency": req.currency,
        "invoice_date": req.invoice_date.to_rfc3339(),
    });

    let bytes = serde_json::to_vec(&canonical).unwrap_or_default();
    let hash = Sha256::digest(&bytes);
    hex::encode(hash)
}

// ============================================================================
// Cache lookup
// ============================================================================

/// Look up a cached tax quote by (app_id, invoice_id) with matching request_hash.
pub async fn find_cached_quote(
    pool: &PgPool,
    app_id: &str,
    invoice_id: &str,
    request_hash: &str,
) -> Result<Option<CachedTaxQuote>, sqlx::Error> {
    let row = sqlx::query_as::<_, (
        Uuid,
        String,
        String,
        String,
        String,
        String,
        String,
        i64,
        serde_json::Value,
        serde_json::Value,
        DateTime<Utc>,
        Option<DateTime<Utc>>,
    )>(
        r#"
        SELECT id, app_id, invoice_id, idempotency_key, request_hash,
               provider, provider_quote_ref, total_tax_minor,
               tax_by_line, response_json, quoted_at, expires_at
        FROM ar_tax_quote_cache
        WHERE app_id = $1 AND invoice_id = $2 AND request_hash = $3
        ORDER BY created_at DESC
        LIMIT 1
        "#,
    )
    .bind(app_id)
    .bind(invoice_id)
    .bind(request_hash)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| CachedTaxQuote {
        id: r.0,
        app_id: r.1,
        invoice_id: r.2,
        idempotency_key: r.3,
        request_hash: r.4,
        provider: r.5,
        provider_quote_ref: r.6,
        total_tax_minor: r.7,
        tax_by_line: r.8,
        response_json: r.9,
        quoted_at: r.10,
        expires_at: r.11,
    }))
}

// ============================================================================
// Cache store
// ============================================================================

/// Store a tax quote response in the cache.
///
/// Uses ON CONFLICT to handle concurrent inserts for the same (app_id, invoice_id, idempotency_key).
pub async fn store_quote_cache(
    pool: &PgPool,
    app_id: &str,
    invoice_id: &str,
    idempotency_key: &str,
    request_hash: &str,
    provider: &str,
    response: &TaxQuoteResponse,
) -> Result<Uuid, sqlx::Error> {
    let response_json = serde_json::to_value(response)
        .unwrap_or_else(|_| serde_json::json!({}));
    let tax_by_line_json = serde_json::to_value(&response.tax_by_line)
        .unwrap_or_else(|_| serde_json::json!([]));

    let id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO ar_tax_quote_cache (
            app_id, invoice_id, idempotency_key, request_hash,
            provider, provider_quote_ref, total_tax_minor,
            tax_by_line, response_json, quoted_at, expires_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        ON CONFLICT (app_id, invoice_id, idempotency_key) DO NOTHING
        RETURNING id
        "#,
    )
    .bind(app_id)
    .bind(invoice_id)
    .bind(idempotency_key)
    .bind(request_hash)
    .bind(provider)
    .bind(&response.provider_quote_ref)
    .bind(response.total_tax_minor)
    .bind(&tax_by_line_json)
    .bind(&response_json)
    .bind(response.quoted_at)
    .bind(response.expires_at)
    .fetch_optional(pool)
    .await?
    .unwrap_or_else(|| {
        // ON CONFLICT fired — the row already exists, return a nil UUID
        // to signal idempotent no-op. Caller can re-query if needed.
        Uuid::nil()
    });

    Ok(id)
}

// ============================================================================
// Cached tax quote service
// ============================================================================

/// Quote tax with cache-through semantics.
///
/// 1. Compute request_hash from the quote request
/// 2. Check ar_tax_quote_cache for a matching (app_id, invoice_id, request_hash)
/// 3. On cache hit: reconstruct TaxQuoteResponse from cached data (deterministic)
/// 4. On cache miss: call provider, persist response, return
///
/// The idempotency_key is derived from (app_id, invoice_id, request_hash) to ensure
/// that the same invoice content always maps to the same cache entry.
pub async fn quote_tax_cached(
    pool: &PgPool,
    provider: &LocalTaxProvider,
    app_id: &str,
    req: TaxQuoteRequest,
) -> Result<TaxQuoteResponse, TaxProviderError> {
    let request_hash = compute_request_hash(&req);
    let invoice_id = req.invoice_id.clone();

    // Check cache
    let cached = find_cached_quote(pool, app_id, &invoice_id, &request_hash)
        .await
        .map_err(|e| TaxProviderError::Provider(format!("cache lookup failed: {}", e)))?;

    if let Some(cached) = cached {
        tracing::debug!(
            app_id = app_id,
            invoice_id = invoice_id.as_str(),
            cache_id = %cached.id,
            "Tax quote cache HIT — returning cached response"
        );

        // Reconstruct response from cached data
        let tax_by_line: Vec<TaxByLine> = serde_json::from_value(cached.tax_by_line)
            .map_err(|e| TaxProviderError::Provider(format!("cached tax_by_line corrupt: {}", e)))?;

        return Ok(TaxQuoteResponse {
            total_tax_minor: cached.total_tax_minor,
            tax_by_line,
            provider_quote_ref: cached.provider_quote_ref,
            expires_at: cached.expires_at,
            quoted_at: cached.quoted_at,
        });
    }

    // Cache miss — call provider
    tracing::debug!(
        app_id = app_id,
        invoice_id = invoice_id.as_str(),
        "Tax quote cache MISS — calling provider"
    );

    let response = provider.quote_tax(req).await?;

    // Persist to cache
    let idempotency_key = format!("{}:{}:{}", app_id, invoice_id, request_hash);
    store_quote_cache(
        pool,
        app_id,
        &invoice_id,
        &idempotency_key,
        &request_hash,
        "local",
        &response,
    )
    .await
    .map_err(|e| TaxProviderError::Provider(format!("cache store failed: {}", e)))?;

    Ok(response)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn sample_address() -> TaxAddress {
        TaxAddress {
            line1: "123 Main St".to_string(),
            line2: None,
            city: "San Francisco".to_string(),
            state: "CA".to_string(),
            postal_code: "94102".to_string(),
            country: "US".to_string(),
        }
    }

    fn sample_line() -> TaxLineItem {
        TaxLineItem {
            line_id: "line-1".to_string(),
            description: "SaaS subscription".to_string(),
            amount_minor: 10000,
            currency: "usd".to_string(),
            tax_code: Some("SW050000".to_string()),
            quantity: 1.0,
        }
    }

    fn sample_quote_req() -> TaxQuoteRequest {
        TaxQuoteRequest {
            tenant_id: "tenant-1".to_string(),
            invoice_id: "inv-1".to_string(),
            customer_id: "cust-1".to_string(),
            ship_to: sample_address(),
            ship_from: sample_address(),
            line_items: vec![sample_line()],
            currency: "usd".to_string(),
            invoice_date: Utc::now(),
            correlation_id: "corr-1".to_string(),
        }
    }

    #[test]
    fn request_hash_is_deterministic() {
        let req = sample_quote_req();
        let h1 = compute_request_hash(&req);
        let h2 = compute_request_hash(&req);
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64); // SHA-256 hex
    }

    #[test]
    fn request_hash_changes_with_amount() {
        let mut req = sample_quote_req();
        let h1 = compute_request_hash(&req);
        req.line_items[0].amount_minor = 20000;
        let h2 = compute_request_hash(&req);
        assert_ne!(h1, h2);
    }
}
