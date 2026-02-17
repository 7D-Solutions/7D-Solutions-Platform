//! Tax provider implementations — ZeroTaxProvider (testing) and LocalTaxProvider (deterministic).

use chrono::Utc;
use uuid::Uuid;

use super::models::*;
use super::{TaxProvider, TaxProviderError};

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
}
