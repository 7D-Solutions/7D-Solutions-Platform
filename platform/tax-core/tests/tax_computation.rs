//! Integration tests — local tax rate lookup and computation via LocalTaxProvider.
//!
//! Tests the full quote_tax / commit_tax / void_tax lifecycle through the public
//! TaxProvider trait, verifying tax amounts, rounding, multi-line breakdowns, and
//! provider_quote_ref format.

use chrono::{DateTime, NaiveDate, Utc};
use tax_core::jurisdiction::{JurisdictionConfig, JurisdictionEntry, TaxRuleConfig};
use tax_core::{
    LocalTaxProvider, TaxAddress, TaxByLine, TaxCommitRequest, TaxLineItem, TaxProvider,
    TaxQuoteRequest, TaxVoidRequest,
};
use uuid::Uuid;

// ── Helpers ────────────────────────────────────────────────────────────

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

fn addr(country: &str, state: &str) -> TaxAddress {
    TaxAddress {
        line1: "1 Test St".into(),
        line2: None,
        city: "Testville".into(),
        state: state.into(),
        postal_code: "00000".into(),
        country: country.into(),
    }
}

fn line(id: &str, amount: i64, tax_code: Option<&str>) -> TaxLineItem {
    TaxLineItem {
        line_id: id.into(),
        description: "Test item".into(),
        amount_minor: amount,
        currency: "usd".into(),
        tax_code: tax_code.map(|s| s.into()),
        quantity: 1.0,
    }
}

fn invoice_date() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-02-01T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

fn quote_req(
    ship_to: TaxAddress,
    ship_from: TaxAddress,
    lines: Vec<TaxLineItem>,
) -> TaxQuoteRequest {
    TaxQuoteRequest {
        tenant_id: "t-1".into(),
        invoice_id: "inv-test".into(),
        customer_id: "c-1".into(),
        ship_to,
        ship_from,
        line_items: lines,
        currency: "usd".into(),
        invoice_date: invoice_date(),
        correlation_id: "corr-test".into(),
    }
}

/// Config with multiple jurisdictions and rate types.
fn multi_config() -> JurisdictionConfig {
    JurisdictionConfig {
        version: "integ-1.0".into(),
        jurisdictions: vec![
            JurisdictionEntry {
                id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
                name: "California Sales Tax".into(),
                country: "US".into(),
                state: Some("CA".into()),
                rules: vec![
                    TaxRuleConfig {
                        tax_type: "sales_tax".into(),
                        rate: 0.085,
                        flat_amount_minor: 0,
                        tax_codes: None,
                        effective_from: date(2025, 1, 1),
                        effective_to: None,
                        is_exempt: false,
                        priority: 10,
                    },
                    TaxRuleConfig {
                        tax_type: "sales_tax".into(),
                        rate: 0.0,
                        flat_amount_minor: 0,
                        tax_codes: Some(vec!["EXEMPT_FOOD".into()]),
                        effective_from: date(2025, 1, 1),
                        effective_to: None,
                        is_exempt: true,
                        priority: 20,
                    },
                ],
            },
            JurisdictionEntry {
                id: Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap(),
                name: "Flat Fee Jurisdiction".into(),
                country: "US".into(),
                state: Some("FL".into()),
                rules: vec![TaxRuleConfig {
                    tax_type: "sales_tax".into(),
                    rate: 0.06,
                    flat_amount_minor: 50, // $0.50 flat + 6% rate
                    tax_codes: None,
                    effective_from: date(2025, 1, 1),
                    effective_to: None,
                    is_exempt: false,
                    priority: 10,
                }],
            },
        ],
    }
}

// ── Tax computation tests ──────────────────────────────────────────────

#[tokio::test]
async fn standard_rate_computation_100_dollars() {
    let provider = LocalTaxProvider::new(multi_config());
    let req = quote_req(addr("US", "CA"), addr("US", "CA"), vec![line("l1", 10000, None)]);
    let resp = provider.quote_tax(req).await.unwrap();

    assert_eq!(resp.total_tax_minor, 850); // 8.5% of $100
    assert_eq!(resp.tax_by_line.len(), 1);
    assert_eq!(resp.tax_by_line[0].tax_minor, 850);
    assert_eq!(resp.tax_by_line[0].rate, 0.085);
    assert_eq!(resp.tax_by_line[0].jurisdiction, "California Sales Tax");
    assert_eq!(resp.tax_by_line[0].tax_type, "sales_tax");
    assert!(resp.warnings.is_empty());
}

#[tokio::test]
async fn exempt_food_line_zero_tax() {
    let provider = LocalTaxProvider::new(multi_config());
    let req = quote_req(
        addr("US", "CA"),
        addr("US", "CA"),
        vec![line("l1", 5000, Some("EXEMPT_FOOD"))],
    );
    let resp = provider.quote_tax(req).await.unwrap();

    assert_eq!(resp.total_tax_minor, 0);
    assert_eq!(resp.tax_by_line[0].tax_minor, 0);
    assert!(resp.tax_by_line[0].tax_type.contains("exempt"));
}

#[tokio::test]
async fn mixed_taxable_and_exempt_lines() {
    let provider = LocalTaxProvider::new(multi_config());
    let req = quote_req(
        addr("US", "CA"),
        addr("US", "CA"),
        vec![
            line("l1", 10000, None),               // taxable: 850
            line("l2", 5000, Some("EXEMPT_FOOD")),  // exempt: 0
            line("l3", 2000, None),                 // taxable: 170
        ],
    );
    let resp = provider.quote_tax(req).await.unwrap();

    assert_eq!(resp.total_tax_minor, 850 + 170);
    assert_eq!(resp.tax_by_line.len(), 3);

    // l1: taxable
    assert_eq!(resp.tax_by_line[0].line_id, "l1");
    assert_eq!(resp.tax_by_line[0].tax_minor, 850);

    // l2: exempt
    assert_eq!(resp.tax_by_line[1].line_id, "l2");
    assert_eq!(resp.tax_by_line[1].tax_minor, 0);

    // l3: taxable
    assert_eq!(resp.tax_by_line[2].line_id, "l3");
    assert_eq!(resp.tax_by_line[2].tax_minor, 170);
}

#[tokio::test]
async fn flat_fee_plus_rate_computation() {
    let provider = LocalTaxProvider::new(multi_config());
    // FL: 6% rate + $0.50 flat (50 minor units)
    let req = quote_req(
        addr("US", "FL"),
        addr("US", "FL"),
        vec![line("l1", 10000, None)],
    );
    let resp = provider.quote_tax(req).await.unwrap();

    // 6% of 10000 = 600, plus flat 50 = 650
    assert_eq!(resp.total_tax_minor, 650);
    assert_eq!(resp.tax_by_line[0].tax_minor, 650);
}

// ── Rounding behaviour ─────────────────────────────────────────────────

#[tokio::test]
async fn rounding_sub_cent_to_zero() {
    let provider = LocalTaxProvider::new(multi_config());
    // 8.5% on 1 cent = 0.085 → rounds to 0
    let req = quote_req(addr("US", "CA"), addr("US", "CA"), vec![line("l1", 1, None)]);
    let resp = provider.quote_tax(req).await.unwrap();
    assert_eq!(resp.total_tax_minor, 0);
}

#[tokio::test]
async fn rounding_half_up_at_boundary() {
    let provider = LocalTaxProvider::new(multi_config());
    // 8.5% on 6 cents = 0.51 → rounds to 1
    let req = quote_req(addr("US", "CA"), addr("US", "CA"), vec![line("l1", 6, None)]);
    let resp = provider.quote_tax(req).await.unwrap();
    assert_eq!(resp.total_tax_minor, 1);
}

#[tokio::test]
async fn rounding_fractional_333_cents() {
    let provider = LocalTaxProvider::new(multi_config());
    // 8.5% on 333 = 28.305 → rounds to 28
    let req = quote_req(addr("US", "CA"), addr("US", "CA"), vec![line("l1", 333, None)]);
    let resp = provider.quote_tax(req).await.unwrap();
    assert_eq!(resp.total_tax_minor, 28);
}

#[tokio::test]
async fn rounding_large_amount() {
    let provider = LocalTaxProvider::new(multi_config());
    // 8.5% on $9,999.99 (999999 minor) = 84999.915 → rounds to 85000
    let req = quote_req(
        addr("US", "CA"),
        addr("US", "CA"),
        vec![line("l1", 999999, None)],
    );
    let resp = provider.quote_tax(req).await.unwrap();
    assert_eq!(resp.total_tax_minor, 85000);
}

// ── Multi-line breakdown ───────────────────────────────────────────────

#[tokio::test]
async fn multi_line_totals_and_ids_correct() {
    let provider = LocalTaxProvider::new(multi_config());
    let req = quote_req(
        addr("US", "CA"),
        addr("US", "CA"),
        vec![
            line("line-a", 10000, None), // 850
            line("line-b", 5000, None),  // 425
            line("line-c", 2000, None),  // 170
            line("line-d", 100, None),   // 9 (8.5 rounds to 9? 100*0.085=8.5 → rounds to 8 or 9?)
        ],
    );
    let resp = provider.quote_tax(req).await.unwrap();

    // 100 * 0.085 = 8.5 → f64::round() = 8.0 (banker's rounding is NOT used, round() is half-up for positive)
    // Actually (100 as f64 * 0.085).round() → 8.5_f64.round() = 8.0 in Rust (round-half-to-even? No, Rust uses round-half-away-from-zero)
    // 8.5_f64.round() = 9.0 in Rust (round half away from zero)
    let expected_d = 9; // 8.5 rounds to 9
    let expected_total = 850 + 425 + 170 + expected_d;

    assert_eq!(resp.total_tax_minor, expected_total);
    assert_eq!(resp.tax_by_line.len(), 4);

    let by_id: std::collections::HashMap<&str, &TaxByLine> = resp
        .tax_by_line
        .iter()
        .map(|t| (t.line_id.as_str(), t))
        .collect();

    assert_eq!(by_id["line-a"].tax_minor, 850);
    assert_eq!(by_id["line-b"].tax_minor, 425);
    assert_eq!(by_id["line-c"].tax_minor, 170);
    assert_eq!(by_id["line-d"].tax_minor, expected_d);
}

// ── Provider quote ref ─────────────────────────────────────────────────

#[tokio::test]
async fn provider_quote_ref_includes_invoice_id_and_version() {
    let provider = LocalTaxProvider::new(multi_config());
    let req = quote_req(addr("US", "CA"), addr("US", "CA"), vec![line("l1", 100, None)]);
    let resp = provider.quote_tax(req).await.unwrap();

    assert!(resp.provider_quote_ref.contains("inv-test"));
    assert!(resp.provider_quote_ref.contains("integ-1.0"));
    assert!(resp.provider_quote_ref.starts_with("local-"));
}

#[tokio::test]
async fn config_version_accessor() {
    let provider = LocalTaxProvider::new(multi_config());
    assert_eq!(provider.config_version(), "integ-1.0");
}

// ── Unknown jurisdiction produces warning ──────────────────────────────

#[tokio::test]
async fn unknown_jurisdiction_zero_tax_with_warning() {
    let provider = LocalTaxProvider::new(multi_config());
    // TX is not configured
    let req = quote_req(
        addr("US", "TX"),
        addr("US", "CA"),
        vec![line("l1", 10000, None)],
    );
    let resp = provider.quote_tax(req).await.unwrap();

    assert_eq!(resp.total_tax_minor, 0);
    assert_eq!(resp.tax_by_line[0].tax_minor, 0);
    assert_eq!(resp.tax_by_line[0].jurisdiction, "not_configured");
    assert_eq!(resp.tax_by_line[0].tax_type, "none");
    assert!(!resp.warnings.is_empty());
    assert!(resp.warnings[0].contains("jurisdiction_not_configured"));
}

// ── Commit / Void lifecycle ────────────────────────────────────────────

#[tokio::test]
async fn commit_returns_reference() {
    let provider = LocalTaxProvider::new(multi_config());
    let resp = provider
        .commit_tax(TaxCommitRequest {
            tenant_id: "t-1".into(),
            invoice_id: "inv-42".into(),
            provider_quote_ref: "local-inv-42-vinteg-1.0".into(),
            correlation_id: "corr-1".into(),
        })
        .await
        .unwrap();

    assert!(resp.provider_commit_ref.contains("inv-42"));
    assert!(resp.provider_commit_ref.starts_with("local-commit-"));
}

#[tokio::test]
async fn void_returns_success() {
    let provider = LocalTaxProvider::new(multi_config());
    let resp = provider
        .void_tax(TaxVoidRequest {
            tenant_id: "t-1".into(),
            invoice_id: "inv-42".into(),
            provider_commit_ref: "local-commit-inv-42".into(),
            void_reason: "customer refund".into(),
            correlation_id: "corr-1".into(),
        })
        .await
        .unwrap();

    assert!(resp.voided);
}

// ── Nexus filtering through provider ───────────────────────────────────

#[tokio::test]
async fn no_nexus_in_destination_produces_warning() {
    let provider = LocalTaxProvider::new(multi_config());
    // ship_from is FL, ship_to is CA — LocalTaxProvider passes ship_from as nexus
    let req = quote_req(
        addr("US", "CA"),
        addr("US", "FL"),
        vec![line("l1", 10000, None)],
    );
    let resp = provider.quote_tax(req).await.unwrap();

    // ship_from (FL) used as nexus address; CA jurisdiction requires nexus in CA
    // Since ship_from is FL, no nexus in CA → unknown → warning
    assert_eq!(resp.total_tax_minor, 0);
    assert!(!resp.warnings.is_empty());
}
