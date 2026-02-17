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
}
