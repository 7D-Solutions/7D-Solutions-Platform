//! ZeroTaxProvider — safe-default provider that always returns zero tax.

use crate::error::TaxProviderError;
use crate::models::*;
use crate::provider::TaxProvider;
use chrono::Utc;

/// Safe-default tax provider that always returns zero tax.
///
/// Used when no tax jurisdiction is configured for a tenant. Every quote
/// includes a `"jurisdiction_not_configured"` warning so callers can detect
/// the zero-tax fallback deterministically.
pub struct ZeroTaxProvider;

impl TaxProvider for ZeroTaxProvider {
    async fn quote_tax(
        &self,
        req: TaxQuoteRequest,
    ) -> Result<TaxQuoteResponse, TaxProviderError> {
        let tax_by_line = req
            .line_items
            .iter()
            .map(|line| TaxByLine {
                line_id: line.line_id.clone(),
                tax_minor: 0,
                rate: 0.0,
                jurisdiction: "not_configured".to_string(),
                tax_type: "none".to_string(),
            })
            .collect();

        Ok(TaxQuoteResponse {
            total_tax_minor: 0,
            tax_by_line,
            provider_quote_ref: format!("zero-tax-{}", req.invoice_id),
            expires_at: None,
            quoted_at: Utc::now(),
            warnings: vec!["jurisdiction_not_configured".to_string()],
        })
    }

    async fn commit_tax(
        &self,
        req: TaxCommitRequest,
    ) -> Result<TaxCommitResponse, TaxProviderError> {
        Ok(TaxCommitResponse {
            provider_commit_ref: format!("zero-commit-{}", req.invoice_id),
            committed_at: Utc::now(),
        })
    }

    async fn void_tax(
        &self,
        _req: TaxVoidRequest,
    ) -> Result<TaxVoidResponse, TaxProviderError> {
        Ok(TaxVoidResponse {
            voided: true,
            voided_at: Utc::now(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{TaxAddress, TaxLineItem};

    fn addr() -> TaxAddress {
        TaxAddress {
            line1: "123 Main St".into(),
            line2: None,
            city: "San Francisco".into(),
            state: "CA".into(),
            postal_code: "94102".into(),
            country: "US".into(),
        }
    }

    fn line(id: &str, amount: i64) -> TaxLineItem {
        TaxLineItem {
            line_id: id.into(),
            description: "Item".into(),
            amount_minor: amount,
            currency: "usd".into(),
            tax_code: None,
            quantity: 1.0,
        }
    }

    fn quote_req(lines: Vec<TaxLineItem>) -> TaxQuoteRequest {
        TaxQuoteRequest {
            tenant_id: "t-1".into(),
            invoice_id: "inv-1".into(),
            customer_id: "c-1".into(),
            ship_to: addr(),
            ship_from: addr(),
            line_items: lines,
            currency: "usd".into(),
            invoice_date: Utc::now(),
            correlation_id: "corr-1".into(),
        }
    }

    #[tokio::test]
    async fn single_line_returns_zero() {
        let p = ZeroTaxProvider;
        let resp = p.quote_tax(quote_req(vec![line("l1", 10000)])).await.unwrap();
        assert_eq!(resp.total_tax_minor, 0);
        assert_eq!(resp.tax_by_line.len(), 1);
        assert_eq!(resp.tax_by_line[0].tax_minor, 0);
        assert_eq!(resp.tax_by_line[0].jurisdiction, "not_configured");
        assert!(resp.warnings.contains(&"jurisdiction_not_configured".to_string()));
    }

    #[tokio::test]
    async fn multiple_lines_all_zero() {
        let p = ZeroTaxProvider;
        let resp = p
            .quote_tax(quote_req(vec![
                line("l1", 5000),
                line("l2", 3000),
                line("l3", 7000),
            ]))
            .await
            .unwrap();
        assert_eq!(resp.total_tax_minor, 0);
        assert_eq!(resp.tax_by_line.len(), 3);
        for tbl in &resp.tax_by_line {
            assert_eq!(tbl.tax_minor, 0);
            assert_eq!(tbl.rate, 0.0);
        }
    }

    #[tokio::test]
    async fn commit_succeeds() {
        let p = ZeroTaxProvider;
        let resp = p
            .commit_tax(TaxCommitRequest {
                tenant_id: "t-1".into(),
                invoice_id: "inv-1".into(),
                provider_quote_ref: "zero-tax-inv-1".into(),
                correlation_id: "corr-1".into(),
            })
            .await
            .unwrap();
        assert!(resp.provider_commit_ref.starts_with("zero-commit-"));
    }

    #[tokio::test]
    async fn void_succeeds() {
        let p = ZeroTaxProvider;
        let resp = p
            .void_tax(TaxVoidRequest {
                tenant_id: "t-1".into(),
                invoice_id: "inv-1".into(),
                provider_commit_ref: "zero-commit-inv-1".into(),
                void_reason: "refund".into(),
                correlation_id: "corr-1".into(),
            })
            .await
            .unwrap();
        assert!(resp.voided);
    }
}
