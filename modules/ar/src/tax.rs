//! Tax Provider Interface (bd-8zm)
//!
//! Defines a provider-agnostic `TaxProvider` trait that Avalara, TaxJar,
//! and local-tax adapters will implement. AR invoice calculation paths
//! call this interface without knowing the underlying provider.
//!
//! ## Lifecycle
//!
//! ```text
//! quote_tax  → provider calculates tax for an invoice draft
//! commit_tax → provider commits tax when invoice is finalized
//! void_tax   → provider voids committed tax on refund/write-off
//! ```
//!
//! ## Determinism
//!
//! Tax calculations MUST be deterministic when using cached provider
//! responses (bd-29j). The provider may be called at most once per
//! invoice; subsequent reads use the cached response.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

// ============================================================================
// Error type
// ============================================================================

#[derive(Debug, Error)]
pub enum TaxProviderError {
    #[error("provider unavailable: {0}")]
    Unavailable(String),
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("commit rejected: {0}")]
    CommitRejected(String),
    #[error("void rejected: {0}")]
    VoidRejected(String),
    #[error("provider error: {0}")]
    Provider(String),
}

// ============================================================================
// Shared value types
// ============================================================================

/// Physical or nexus address for jurisdiction resolution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaxAddress {
    pub line1: String,
    pub line2: Option<String>,
    pub city: String,
    /// State/province code (ISO 3166-2 subdivision)
    pub state: String,
    /// Postal/ZIP code
    pub postal_code: String,
    /// ISO 3166-1 alpha-2 country code
    pub country: String,
}

/// A single taxable line on an invoice
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaxLineItem {
    /// Corresponds to invoice line item id or usage metric
    pub line_id: String,
    pub description: String,
    /// Taxable amount in minor currency units (e.g. cents)
    pub amount_minor: i64,
    pub currency: String,
    /// Provider-specific product/tax-code (e.g. "SW050000" for SaaS)
    pub tax_code: Option<String>,
    /// Quantity (for unit-based tax regimes)
    pub quantity: f64,
}

/// Tax applied to a single line item (from provider response)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaxByLine {
    pub line_id: String,
    /// Tax amount for this line in minor currency units
    pub tax_minor: i64,
    /// Effective tax rate (0.0–1.0)
    pub rate: f64,
    /// Tax jurisdiction name (e.g. "California State Tax")
    pub jurisdiction: String,
    /// Tax type (e.g. "sales_tax", "vat", "gst")
    pub tax_type: String,
}

// ============================================================================
// quote_tax
// ============================================================================

/// Request a tax calculation for an invoice draft.
/// The provider MUST NOT commit any tax at this stage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaxQuoteRequest {
    pub tenant_id: String,
    pub invoice_id: String,
    pub customer_id: String,
    /// Destination address (customer's billing address)
    pub ship_to: TaxAddress,
    /// Origin address (seller's address / nexus)
    pub ship_from: TaxAddress,
    pub line_items: Vec<TaxLineItem>,
    pub currency: String,
    pub invoice_date: DateTime<Utc>,
    /// Correlation ID for tracing (passed through to provider if supported)
    pub correlation_id: String,
}

/// Provider response to a tax quote request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaxQuoteResponse {
    /// Total tax across all lines in minor currency units
    pub total_tax_minor: i64,
    /// Per-line tax breakdown
    pub tax_by_line: Vec<TaxByLine>,
    /// Provider-assigned reference for this quote (used to commit/void)
    pub provider_quote_ref: String,
    /// When this quote expires (provider may require re-quote after this)
    pub expires_at: Option<DateTime<Utc>>,
    pub quoted_at: DateTime<Utc>,
}

// ============================================================================
// commit_tax
// ============================================================================

/// Commit a previously quoted tax calculation.
/// Called when an invoice is finalized and tax is legally due.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaxCommitRequest {
    pub tenant_id: String,
    pub invoice_id: String,
    /// Quote reference from a prior quote_tax call
    pub provider_quote_ref: String,
    pub correlation_id: String,
}

/// Provider acknowledgment of a committed tax transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaxCommitResponse {
    /// Provider-assigned reference for the committed transaction (for void)
    pub provider_commit_ref: String,
    pub committed_at: DateTime<Utc>,
}

// ============================================================================
// void_tax
// ============================================================================

/// Void a committed tax transaction.
/// Called on full refund, write-off, or invoice cancellation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaxVoidRequest {
    pub tenant_id: String,
    pub invoice_id: String,
    /// Commit reference from a prior commit_tax call
    pub provider_commit_ref: String,
    pub void_reason: String,
    pub correlation_id: String,
}

/// Provider acknowledgment of a voided tax transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaxVoidResponse {
    pub voided: bool,
    pub voided_at: DateTime<Utc>,
}

// ============================================================================
// TaxProvider trait
// ============================================================================

/// Provider-agnostic tax interface.
///
/// Implementations: Avalara, TaxJar, local-tax (bd-29j), etc.
///
/// Implementors MUST be `Send + Sync` (shared across Tokio tasks).
/// All methods are async and MUST NOT block.
///
/// Error handling: providers should return `TaxProviderError::Unavailable`
/// for transient failures so callers can apply retry/circuit-breaker logic.
pub trait TaxProvider: Send + Sync {
    fn quote_tax(
        &self,
        req: TaxQuoteRequest,
    ) -> impl std::future::Future<Output = Result<TaxQuoteResponse, TaxProviderError>> + Send;

    fn commit_tax(
        &self,
        req: TaxCommitRequest,
    ) -> impl std::future::Future<Output = Result<TaxCommitResponse, TaxProviderError>> + Send;

    fn void_tax(
        &self,
        req: TaxVoidRequest,
    ) -> impl std::future::Future<Output = Result<TaxVoidResponse, TaxProviderError>> + Send;
}

// ============================================================================
// Stub implementation for testing
// ============================================================================

/// No-op tax provider that returns zero tax for all requests.
/// Used in tests and local development where no live provider is configured.
pub struct ZeroTaxProvider;

impl TaxProvider for ZeroTaxProvider {
    async fn quote_tax(&self, req: TaxQuoteRequest) -> Result<TaxQuoteResponse, TaxProviderError> {
        let zero_lines: Vec<TaxByLine> = req
            .line_items
            .iter()
            .map(|l| TaxByLine {
                line_id: l.line_id.clone(),
                tax_minor: 0,
                rate: 0.0,
                jurisdiction: "zero-tax".to_string(),
                tax_type: "none".to_string(),
            })
            .collect();

        Ok(TaxQuoteResponse {
            total_tax_minor: 0,
            tax_by_line: zero_lines,
            provider_quote_ref: format!("zero-quote-{}", Uuid::new_v4()),
            expires_at: None,
            quoted_at: Utc::now(),
        })
    }

    async fn commit_tax(
        &self,
        _req: TaxCommitRequest,
    ) -> Result<TaxCommitResponse, TaxProviderError> {
        Ok(TaxCommitResponse {
            provider_commit_ref: format!("zero-commit-{}", Uuid::new_v4()),
            committed_at: Utc::now(),
        })
    }

    async fn void_tax(&self, _req: TaxVoidRequest) -> Result<TaxVoidResponse, TaxProviderError> {
        Ok(TaxVoidResponse {
            voided: true,
            voided_at: Utc::now(),
        })
    }
}

// ============================================================================
// Local deterministic tax provider (bd-29j)
// ============================================================================

/// Deterministic tax provider for E2E testing and local development.
///
/// Calculates tax based on the destination state using a fixed rate table.
/// Rates are hardcoded to ensure deterministic, reproducible results across
/// test runs without requiring an external tax service.
///
/// Rate table (US states, ship_to.state):
/// - CA: 8.5% (California)
/// - NY: 8.0% (New York)
/// - TX: 6.25% (Texas)
/// - WA: 6.5% (Washington)
/// - FL: 6.0% (Florida)
/// - All others: 5.0% (default)
///
/// Non-US countries: 0% (tax-exempt in local provider)
pub struct LocalTaxProvider;

impl LocalTaxProvider {
    /// Resolve the tax rate for a given state code.
    /// Returns (rate, jurisdiction_name).
    fn resolve_rate(state: &str, country: &str) -> (f64, String) {
        if country != "US" {
            return (0.0, format!("{} (exempt)", country));
        }
        match state.to_uppercase().as_str() {
            "CA" => (0.085, "California State Tax".to_string()),
            "NY" => (0.08, "New York State Tax".to_string()),
            "TX" => (0.0625, "Texas State Tax".to_string()),
            "WA" => (0.065, "Washington State Tax".to_string()),
            "FL" => (0.06, "Florida State Tax".to_string()),
            other => (0.05, format!("{} Default Tax", other)),
        }
    }
}

impl TaxProvider for LocalTaxProvider {
    async fn quote_tax(&self, req: TaxQuoteRequest) -> Result<TaxQuoteResponse, TaxProviderError> {
        if req.line_items.is_empty() {
            return Err(TaxProviderError::InvalidRequest(
                "No line items provided".to_string(),
            ));
        }

        let (rate, jurisdiction) =
            Self::resolve_rate(&req.ship_to.state, &req.ship_to.country);

        let mut total_tax: i64 = 0;
        let tax_by_line: Vec<TaxByLine> = req
            .line_items
            .iter()
            .map(|l| {
                // Banker's rounding: (amount * rate + 0.5).floor()
                let tax = ((l.amount_minor as f64) * rate).round() as i64;
                total_tax += tax;
                TaxByLine {
                    line_id: l.line_id.clone(),
                    tax_minor: tax,
                    rate,
                    jurisdiction: jurisdiction.clone(),
                    tax_type: "sales_tax".to_string(),
                }
            })
            .collect();

        Ok(TaxQuoteResponse {
            total_tax_minor: total_tax,
            tax_by_line,
            provider_quote_ref: format!("local-quote-{}", Uuid::new_v4()),
            expires_at: None,
            quoted_at: Utc::now(),
        })
    }

    async fn commit_tax(
        &self,
        req: TaxCommitRequest,
    ) -> Result<TaxCommitResponse, TaxProviderError> {
        if !req.provider_quote_ref.starts_with("local-quote-") {
            return Err(TaxProviderError::CommitRejected(
                "Unknown quote reference".to_string(),
            ));
        }
        Ok(TaxCommitResponse {
            provider_commit_ref: format!(
                "local-commit-{}",
                Uuid::new_v4()
            ),
            committed_at: Utc::now(),
        })
    }

    async fn void_tax(&self, req: TaxVoidRequest) -> Result<TaxVoidResponse, TaxProviderError> {
        if !req.provider_commit_ref.starts_with("local-commit-") {
            return Err(TaxProviderError::VoidRejected(
                "Unknown commit reference".to_string(),
            ));
        }
        Ok(TaxVoidResponse {
            voided: true,
            voided_at: Utc::now(),
        })
    }
}

// ============================================================================
// Tax quote cache (bd-29j)
// ============================================================================

use sqlx::PgPool;

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
// Cached tax quote service (bd-29j)
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

    #[tokio::test]
    async fn zero_tax_provider_returns_zero_tax() {
        let provider = ZeroTaxProvider;
        let response = provider.quote_tax(sample_quote_req()).await.unwrap();
        assert_eq!(response.total_tax_minor, 0);
        assert_eq!(response.tax_by_line.len(), 1);
        assert_eq!(response.tax_by_line[0].tax_minor, 0);
        assert_eq!(response.tax_by_line[0].rate, 0.0);
        assert!(!response.provider_quote_ref.is_empty());
    }

    #[tokio::test]
    async fn zero_tax_provider_commit_succeeds() {
        let provider = ZeroTaxProvider;
        let req = TaxCommitRequest {
            tenant_id: "tenant-1".to_string(),
            invoice_id: "inv-1".to_string(),
            provider_quote_ref: "quote-abc".to_string(),
            correlation_id: "corr-1".to_string(),
        };
        let resp = provider.commit_tax(req).await.unwrap();
        assert!(!resp.provider_commit_ref.is_empty());
    }

    #[tokio::test]
    async fn zero_tax_provider_void_succeeds() {
        let provider = ZeroTaxProvider;
        let req = TaxVoidRequest {
            tenant_id: "tenant-1".to_string(),
            invoice_id: "inv-1".to_string(),
            provider_commit_ref: "commit-abc".to_string(),
            void_reason: "invoice_cancelled".to_string(),
            correlation_id: "corr-1".to_string(),
        };
        let resp = provider.void_tax(req).await.unwrap();
        assert!(resp.voided);
    }

    #[test]
    fn tax_address_serializes() {
        let addr = sample_address();
        let json = serde_json::to_string(&addr).unwrap();
        assert!(json.contains("San Francisco"));
        assert!(json.contains("postal_code"));
    }

    #[test]
    fn tax_line_item_serializes() {
        let line = sample_line();
        let json = serde_json::to_string(&line).unwrap();
        assert!(json.contains("SW050000"));
        assert!(json.contains("amount_minor"));
    }

    #[test]
    fn tax_quote_request_serializes() {
        let req = sample_quote_req();
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("invoice_id"));
        assert!(json.contains("ship_to"));
        assert!(json.contains("ship_from"));
        assert!(json.contains("line_items"));
    }

    // ========================================================================
    // LocalTaxProvider tests (bd-29j)
    // ========================================================================

    #[tokio::test]
    async fn local_provider_california_rate() {
        let provider = LocalTaxProvider;
        let mut req = sample_quote_req();
        req.ship_to.state = "CA".to_string();
        let resp = provider.quote_tax(req).await.unwrap();
        // 10000 * 0.085 = 850
        assert_eq!(resp.total_tax_minor, 850);
        assert_eq!(resp.tax_by_line[0].rate, 0.085);
        assert_eq!(resp.tax_by_line[0].jurisdiction, "California State Tax");
        assert!(resp.provider_quote_ref.starts_with("local-quote-"));
    }

    #[tokio::test]
    async fn local_provider_new_york_rate() {
        let provider = LocalTaxProvider;
        let mut req = sample_quote_req();
        req.ship_to.state = "NY".to_string();
        let resp = provider.quote_tax(req).await.unwrap();
        // 10000 * 0.08 = 800
        assert_eq!(resp.total_tax_minor, 800);
        assert_eq!(resp.tax_by_line[0].rate, 0.08);
    }

    #[tokio::test]
    async fn local_provider_default_rate() {
        let provider = LocalTaxProvider;
        let mut req = sample_quote_req();
        req.ship_to.state = "MT".to_string(); // Montana — not in rate table
        let resp = provider.quote_tax(req).await.unwrap();
        // 10000 * 0.05 = 500
        assert_eq!(resp.total_tax_minor, 500);
        assert_eq!(resp.tax_by_line[0].rate, 0.05);
    }

    #[tokio::test]
    async fn local_provider_non_us_exempt() {
        let provider = LocalTaxProvider;
        let mut req = sample_quote_req();
        req.ship_to.country = "GB".to_string();
        let resp = provider.quote_tax(req).await.unwrap();
        assert_eq!(resp.total_tax_minor, 0);
        assert_eq!(resp.tax_by_line[0].rate, 0.0);
    }

    #[tokio::test]
    async fn local_provider_multi_line() {
        let provider = LocalTaxProvider;
        let mut req = sample_quote_req();
        req.ship_to.state = "CA".to_string();
        req.line_items.push(TaxLineItem {
            line_id: "line-2".to_string(),
            description: "Storage addon".to_string(),
            amount_minor: 5000,
            currency: "usd".to_string(),
            tax_code: None,
            quantity: 1.0,
        });
        let resp = provider.quote_tax(req).await.unwrap();
        // 10000 * 0.085 = 850, 5000 * 0.085 = 425 → total 1275
        assert_eq!(resp.total_tax_minor, 1275);
        assert_eq!(resp.tax_by_line.len(), 2);
    }

    #[tokio::test]
    async fn local_provider_empty_lines_rejected() {
        let provider = LocalTaxProvider;
        let mut req = sample_quote_req();
        req.line_items.clear();
        let err = provider.quote_tax(req).await.unwrap_err();
        assert!(matches!(err, TaxProviderError::InvalidRequest(_)));
    }

    #[tokio::test]
    async fn local_provider_commit_rejects_unknown_ref() {
        let provider = LocalTaxProvider;
        let req = TaxCommitRequest {
            tenant_id: "t".to_string(),
            invoice_id: "i".to_string(),
            provider_quote_ref: "avalara-quote-123".to_string(),
            correlation_id: "c".to_string(),
        };
        let err = provider.commit_tax(req).await.unwrap_err();
        assert!(matches!(err, TaxProviderError::CommitRejected(_)));
    }

    #[tokio::test]
    async fn local_provider_void_rejects_unknown_ref() {
        let provider = LocalTaxProvider;
        let req = TaxVoidRequest {
            tenant_id: "t".to_string(),
            invoice_id: "i".to_string(),
            provider_commit_ref: "avalara-commit-123".to_string(),
            void_reason: "test".to_string(),
            correlation_id: "c".to_string(),
        };
        let err = provider.void_tax(req).await.unwrap_err();
        assert!(matches!(err, TaxProviderError::VoidRejected(_)));
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
