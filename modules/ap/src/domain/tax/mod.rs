//! AP tax domain — quote/commit/void lifecycle for vendor bill tax.
//!
//! Uses the shared `tax-core` TaxProvider trait and types.
//! AP persists tax snapshots per bill for audit and idempotency.

pub mod models;
pub mod service;

pub use models::ApTaxSnapshot;
pub use service::{ApTaxError, commit_bill_tax, find_active_snapshot, quote_bill_tax, void_bill_tax};

// Re-export shared types for convenience
pub use tax_core::models::TaxAddress;
pub use tax_core::TaxProvider;

// ---------------------------------------------------------------------------
// ZeroTaxProvider — default provider for AP (returns zero tax for all requests)
// ---------------------------------------------------------------------------

use chrono::Utc;
use uuid::Uuid;

use tax_core::models::*;
use tax_core::TaxProviderError;

/// No-op tax provider returning zero tax. Used as AP default when no external
/// tax service is configured.
pub struct ZeroTaxProvider;

impl tax_core::TaxProvider for ZeroTaxProvider {
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

#[cfg(test)]
mod tests {
    use super::*;

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

    fn sample_quote_req() -> TaxQuoteRequest {
        TaxQuoteRequest {
            tenant_id: "t1".to_string(),
            invoice_id: "bill-1".to_string(),
            customer_id: "vendor-1".to_string(),
            ship_to: sample_address(),
            ship_from: sample_address(),
            line_items: vec![TaxLineItem {
                line_id: "line-1".to_string(),
                description: "Widget".to_string(),
                amount_minor: 10000,
                currency: "USD".to_string(),
                tax_code: None,
                quantity: 1.0,
            }],
            currency: "USD".to_string(),
            invoice_date: Utc::now(),
            correlation_id: "corr-1".to_string(),
        }
    }

    #[tokio::test]
    async fn zero_tax_provider_returns_zero() {
        let provider = ZeroTaxProvider;
        let resp = provider.quote_tax(sample_quote_req()).await.unwrap();
        assert_eq!(resp.total_tax_minor, 0);
        assert_eq!(resp.tax_by_line.len(), 1);
        assert!(resp.provider_quote_ref.starts_with("zero-quote-"));
    }

    #[tokio::test]
    async fn zero_tax_provider_commit_succeeds() {
        let provider = ZeroTaxProvider;
        let req = TaxCommitRequest {
            tenant_id: "t1".to_string(),
            invoice_id: "bill-1".to_string(),
            provider_quote_ref: "any-ref".to_string(),
            correlation_id: "c1".to_string(),
        };
        let resp = provider.commit_tax(req).await.unwrap();
        assert!(resp.provider_commit_ref.starts_with("zero-commit-"));
    }

    #[tokio::test]
    async fn zero_tax_provider_void_succeeds() {
        let provider = ZeroTaxProvider;
        let req = TaxVoidRequest {
            tenant_id: "t1".to_string(),
            invoice_id: "bill-1".to_string(),
            provider_commit_ref: "any-ref".to_string(),
            void_reason: "bill voided".to_string(),
            correlation_id: "c1".to_string(),
        };
        let resp = provider.void_tax(req).await.unwrap();
        assert!(resp.voided);
    }
}
