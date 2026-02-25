//! LocalTaxProvider — computes tax from static jurisdiction config.

use crate::error::TaxProviderError;
use crate::jurisdiction::{resolve_jurisdiction, JurisdictionConfig, JurisdictionResult};
use crate::models::*;
use crate::provider::TaxProvider;
use chrono::Utc;

/// Local tax provider that computes tax deterministically from static config.
///
/// Uses [`resolve_jurisdiction`] to find applicable rules, then applies rates.
/// Every response includes the config `version` in `provider_quote_ref` for
/// reproducibility.
pub struct LocalTaxProvider {
    config: JurisdictionConfig,
}

impl LocalTaxProvider {
    pub fn new(config: JurisdictionConfig) -> Self {
        Self { config }
    }

    pub fn config_version(&self) -> &str {
        &self.config.version
    }
}

/// Compute tax for a single line item given resolved rules.
fn compute_line_tax(line: &TaxLineItem, rules: &[ResolvedRule]) -> (i64, Vec<TaxByLine>) {
    let mut total_tax = 0i64;
    let mut breakdowns = Vec::new();

    for rule in rules {
        if rule.is_exempt {
            breakdowns.push(TaxByLine {
                line_id: line.line_id.clone(),
                tax_minor: 0,
                rate: 0.0,
                jurisdiction: rule.jurisdiction_name.clone(),
                tax_type: format!("{}_exempt", rule.tax_type),
            });
            continue;
        }

        // Rate-based tax: round half-away-from-zero on minor-unit result
        let rate_tax = (line.amount_minor as f64 * rule.rate).round() as i64;
        let line_tax = rate_tax + rule.flat_amount_minor;
        total_tax += line_tax;

        breakdowns.push(TaxByLine {
            line_id: line.line_id.clone(),
            tax_minor: line_tax,
            rate: rule.rate,
            jurisdiction: rule.jurisdiction_name.clone(),
            tax_type: rule.tax_type.clone(),
        });
    }

    (total_tax, breakdowns)
}

impl TaxProvider for LocalTaxProvider {
    async fn quote_tax(
        &self,
        req: TaxQuoteRequest,
    ) -> Result<TaxQuoteResponse, TaxProviderError> {
        let as_of = req.invoice_date.date_naive();
        let mut total_tax = 0i64;
        let mut all_breakdowns = Vec::new();
        let mut warnings = Vec::new();

        for line in &req.line_items {
            let result = resolve_jurisdiction(
                &req.ship_to,
                &req.ship_to, // bill_to = ship_to (same field in request)
                std::slice::from_ref(&req.ship_from),
                line.tax_code.as_deref(),
                as_of,
                &self.config,
            );

            match result {
                JurisdictionResult::Resolved { rules, .. } => {
                    let (line_tax, breakdowns) = compute_line_tax(line, &rules);
                    total_tax += line_tax;
                    all_breakdowns.extend(breakdowns);
                }
                JurisdictionResult::Unknown => {
                    warnings.push(format!(
                        "jurisdiction_not_configured: line={}, country={}, state={}",
                        line.line_id, req.ship_to.country, req.ship_to.state
                    ));
                    all_breakdowns.push(TaxByLine {
                        line_id: line.line_id.clone(),
                        tax_minor: 0,
                        rate: 0.0,
                        jurisdiction: "not_configured".to_string(),
                        tax_type: "none".to_string(),
                    });
                }
            }
        }

        Ok(TaxQuoteResponse {
            total_tax_minor: total_tax,
            tax_by_line: all_breakdowns,
            provider_quote_ref: format!(
                "local-{}-v{}",
                req.invoice_id, self.config.version
            ),
            expires_at: None,
            quoted_at: Utc::now(),
            warnings,
        })
    }

    async fn commit_tax(
        &self,
        req: TaxCommitRequest,
    ) -> Result<TaxCommitResponse, TaxProviderError> {
        Ok(TaxCommitResponse {
            provider_commit_ref: format!("local-commit-{}", req.invoice_id),
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
    use crate::jurisdiction::{JurisdictionEntry, TaxRuleConfig};
    use chrono::NaiveDate;
    use uuid::Uuid;

    fn ca_config() -> JurisdictionConfig {
        let ca_id = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        JurisdictionConfig {
            version: "test-1.0".into(),
            jurisdictions: vec![JurisdictionEntry {
                id: ca_id,
                name: "California State Tax".into(),
                country: "US".into(),
                state: Some("CA".into()),
                rules: vec![
                    TaxRuleConfig {
                        tax_type: "sales_tax".into(),
                        rate: 0.085,
                        flat_amount_minor: 0,
                        tax_codes: None,
                        effective_from: NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
                        effective_to: None,
                        is_exempt: false,
                        priority: 10,
                    },
                    TaxRuleConfig {
                        tax_type: "sales_tax".into(),
                        rate: 0.0,
                        flat_amount_minor: 0,
                        tax_codes: Some(vec!["EXEMPT_FOOD".into()]),
                        effective_from: NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
                        effective_to: None,
                        is_exempt: true,
                        priority: 20,
                    },
                ],
            }],
        }
    }

    fn ca_addr() -> TaxAddress {
        TaxAddress {
            line1: "123 Main St".into(),
            line2: None,
            city: "San Francisco".into(),
            state: "CA".into(),
            postal_code: "94102".into(),
            country: "US".into(),
        }
    }

    fn tx_addr() -> TaxAddress {
        TaxAddress {
            line1: "456 Elm St".into(),
            line2: None,
            city: "Austin".into(),
            state: "TX".into(),
            postal_code: "73301".into(),
            country: "US".into(),
        }
    }

    fn line(id: &str, amount: i64, tax_code: Option<&str>) -> TaxLineItem {
        TaxLineItem {
            line_id: id.into(),
            description: "Test item".into(),
            amount_minor: amount,
            currency: "usd".into(),
            tax_code: tax_code.map(|s| s.to_string()),
            quantity: 1.0,
        }
    }

    fn quote(
        ship_to: TaxAddress,
        ship_from: TaxAddress,
        lines: Vec<TaxLineItem>,
    ) -> TaxQuoteRequest {
        TaxQuoteRequest {
            tenant_id: "t-1".into(),
            invoice_id: "inv-1".into(),
            customer_id: "c-1".into(),
            ship_to,
            ship_from,
            line_items: lines,
            currency: "usd".into(),
            invoice_date: chrono::DateTime::parse_from_rfc3339("2026-02-01T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            correlation_id: "corr-1".into(),
        }
    }

    // ── Test vectors ──────────────────────────────────────────────────

    #[tokio::test]
    async fn ca_8_5_pct_on_100_dollars() {
        // 8.5% on 10000 minor (=$100) → 850 minor (=$8.50)
        let p = LocalTaxProvider::new(ca_config());
        let req = quote(ca_addr(), ca_addr(), vec![line("l1", 10000, None)]);
        let resp = p.quote_tax(req).await.unwrap();
        assert_eq!(resp.total_tax_minor, 850);
        assert_eq!(resp.tax_by_line[0].tax_minor, 850);
        assert_eq!(resp.tax_by_line[0].rate, 0.085);
        assert!(resp.warnings.is_empty());
    }

    #[tokio::test]
    async fn exempt_line_zero_tax() {
        let p = LocalTaxProvider::new(ca_config());
        let req = quote(
            ca_addr(),
            ca_addr(),
            vec![line("l1", 5000, Some("EXEMPT_FOOD"))],
        );
        let resp = p.quote_tax(req).await.unwrap();
        assert_eq!(resp.total_tax_minor, 0);
        assert_eq!(resp.tax_by_line[0].tax_minor, 0);
        assert!(resp.tax_by_line[0].tax_type.contains("exempt"));
    }

    #[tokio::test]
    async fn rounding_half_up() {
        // 8.5% on 1 cent = 0.085 → rounds to 0
        let p = LocalTaxProvider::new(ca_config());
        let req = quote(ca_addr(), ca_addr(), vec![line("l1", 1, None)]);
        let resp = p.quote_tax(req).await.unwrap();
        assert_eq!(resp.total_tax_minor, 0);
    }

    #[tokio::test]
    async fn rounding_at_boundary() {
        // 8.5% on 6 cents = 0.51 → rounds to 1
        let p = LocalTaxProvider::new(ca_config());
        let req = quote(ca_addr(), ca_addr(), vec![line("l1", 6, None)]);
        let resp = p.quote_tax(req).await.unwrap();
        assert_eq!(resp.total_tax_minor, 1);
    }

    #[tokio::test]
    async fn rounding_fractional_amount() {
        // 8.5% on 333 cents = 28.305 → rounds to 28
        let p = LocalTaxProvider::new(ca_config());
        let req = quote(ca_addr(), ca_addr(), vec![line("l1", 333, None)]);
        let resp = p.quote_tax(req).await.unwrap();
        assert_eq!(resp.total_tax_minor, 28);
    }

    #[tokio::test]
    async fn multi_line_breakdown() {
        let p = LocalTaxProvider::new(ca_config());
        let req = quote(
            ca_addr(),
            ca_addr(),
            vec![
                line("l1", 10000, None), // 850
                line("l2", 5000, None),  // 425
                line("l3", 2000, None),  // 170
            ],
        );
        let resp = p.quote_tax(req).await.unwrap();
        assert_eq!(resp.total_tax_minor, 850 + 425 + 170);
        assert_eq!(resp.tax_by_line.len(), 3);
        assert_eq!(resp.tax_by_line[0].tax_minor, 850);
        assert_eq!(resp.tax_by_line[1].tax_minor, 425);
        assert_eq!(resp.tax_by_line[2].tax_minor, 170);
    }

    #[tokio::test]
    async fn unknown_jurisdiction_zero_with_warning() {
        let p = LocalTaxProvider::new(ca_config());
        let req = quote(tx_addr(), ca_addr(), vec![line("l1", 10000, None)]);
        let resp = p.quote_tax(req).await.unwrap();
        assert_eq!(resp.total_tax_minor, 0);
        assert_eq!(resp.tax_by_line[0].jurisdiction, "not_configured");
        assert!(!resp.warnings.is_empty());
        assert!(resp.warnings[0].contains("jurisdiction_not_configured"));
    }

    #[tokio::test]
    async fn provider_quote_ref_includes_version() {
        let p = LocalTaxProvider::new(ca_config());
        let req = quote(ca_addr(), ca_addr(), vec![line("l1", 100, None)]);
        let resp = p.quote_tax(req).await.unwrap();
        assert!(resp.provider_quote_ref.contains("test-1.0"));
    }
}
