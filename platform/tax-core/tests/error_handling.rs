//! Integration tests — error handling for unknown jurisdictions and edge cases.
//!
//! Verifies graceful degradation when jurisdiction config is empty, addresses
//! don't match, or inputs exercise boundary conditions.

use chrono::{DateTime, Utc};
use tax_core::jurisdiction::{JurisdictionConfig, JurisdictionEntry, TaxRuleConfig};
use tax_core::{
    LocalTaxProvider, TaxAddress, TaxLineItem, TaxProvider, TaxProviderError, TaxQuoteRequest,
};
use uuid::Uuid;

// ── Helpers ────────────────────────────────────────────────────────────

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
        tenant_id: "t-err".into(),
        invoice_id: "inv-err".into(),
        customer_id: "c-err".into(),
        ship_to,
        ship_from,
        line_items: lines,
        currency: "usd".into(),
        invoice_date: invoice_date(),
        correlation_id: "corr-err".into(),
    }
}

fn empty_config() -> JurisdictionConfig {
    JurisdictionConfig {
        version: "empty".into(),
        jurisdictions: vec![],
    }
}

fn ca_only_config() -> JurisdictionConfig {
    JurisdictionConfig {
        version: "ca-only".into(),
        jurisdictions: vec![JurisdictionEntry {
            id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
            name: "California Sales Tax".into(),
            country: "US".into(),
            state: Some("CA".into()),
            rules: vec![TaxRuleConfig {
                tax_type: "sales_tax".into(),
                rate: 0.085,
                flat_amount_minor: 0,
                tax_codes: None,
                effective_from: chrono::NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
                effective_to: None,
                is_exempt: false,
                priority: 10,
            }],
        }],
    }
}

// ── Empty config ───────────────────────────────────────────────────────

#[tokio::test]
async fn empty_config_returns_zero_with_warning() {
    let provider = LocalTaxProvider::new(empty_config());
    let req = quote_req(addr("US", "CA"), addr("US", "CA"), vec![line("l1", 10000)]);
    let resp = provider.quote_tax(req).await.unwrap();

    assert_eq!(resp.total_tax_minor, 0);
    assert_eq!(resp.tax_by_line[0].jurisdiction, "not_configured");
    assert!(!resp.warnings.is_empty());
    assert!(resp.warnings[0].contains("jurisdiction_not_configured"));
}

// ── Unknown country / state ────────────────────────────────────────────

#[tokio::test]
async fn unknown_country_graceful_zero() {
    let provider = LocalTaxProvider::new(ca_only_config());
    let req = quote_req(addr("JP", "TK"), addr("US", "CA"), vec![line("l1", 10000)]);
    let resp = provider.quote_tax(req).await.unwrap();

    assert_eq!(resp.total_tax_minor, 0);
    assert!(resp.warnings[0].contains("JP"));
}

#[tokio::test]
async fn unknown_state_graceful_zero() {
    let provider = LocalTaxProvider::new(ca_only_config());
    let req = quote_req(addr("US", "TX"), addr("US", "CA"), vec![line("l1", 10000)]);
    let resp = provider.quote_tax(req).await.unwrap();

    assert_eq!(resp.total_tax_minor, 0);
    assert!(resp.warnings[0].contains("TX"));
}

// ── Multiple unknown lines ─────────────────────────────────────────────

#[tokio::test]
async fn multiple_lines_all_unknown_all_warned() {
    let provider = LocalTaxProvider::new(ca_only_config());
    let req = quote_req(
        addr("US", "TX"),
        addr("US", "CA"),
        vec![line("l1", 5000), line("l2", 3000), line("l3", 7000)],
    );
    let resp = provider.quote_tax(req).await.unwrap();

    assert_eq!(resp.total_tax_minor, 0);
    assert_eq!(resp.tax_by_line.len(), 3);
    // Each line should have a warning
    assert_eq!(resp.warnings.len(), 3);
    for tbl in &resp.tax_by_line {
        assert_eq!(tbl.tax_minor, 0);
        assert_eq!(tbl.jurisdiction, "not_configured");
        assert_eq!(tbl.tax_type, "none");
    }
}

// ── Error type display ─────────────────────────────────────────────────

#[test]
fn error_display_unavailable() {
    let err = TaxProviderError::Unavailable("service down".into());
    assert_eq!(format!("{err}"), "provider unavailable: service down");
}

#[test]
fn error_display_invalid_request() {
    let err = TaxProviderError::InvalidRequest("missing field".into());
    assert_eq!(format!("{err}"), "invalid request: missing field");
}

#[test]
fn error_display_commit_rejected() {
    let err = TaxProviderError::CommitRejected("quote expired".into());
    assert_eq!(format!("{err}"), "commit rejected: quote expired");
}

#[test]
fn error_display_void_rejected() {
    let err = TaxProviderError::VoidRejected("already voided".into());
    assert_eq!(format!("{err}"), "void rejected: already voided");
}

#[test]
fn error_display_provider() {
    let err = TaxProviderError::Provider("internal error".into());
    assert_eq!(format!("{err}"), "provider error: internal error");
}

#[test]
fn error_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<TaxProviderError>();
}

// ── Zero-amount line ───────────────────────────────────────────────────

#[tokio::test]
async fn zero_amount_line_produces_zero_tax() {
    let provider = LocalTaxProvider::new(ca_only_config());
    let req = quote_req(addr("US", "CA"), addr("US", "CA"), vec![line("l1", 0)]);
    let resp = provider.quote_tax(req).await.unwrap();

    assert_eq!(resp.total_tax_minor, 0);
    assert_eq!(resp.tax_by_line[0].tax_minor, 0);
    assert!(resp.warnings.is_empty()); // jurisdiction IS configured, just zero amount
}

// ── Negative amount (credit) ───────────────────────────────────────────

#[tokio::test]
async fn negative_amount_produces_negative_tax() {
    let provider = LocalTaxProvider::new(ca_only_config());
    // Credit line: -$100 = -10000 minor
    let req = quote_req(addr("US", "CA"), addr("US", "CA"), vec![line("l1", -10000)]);
    let resp = provider.quote_tax(req).await.unwrap();

    // 8.5% of -10000 = -850
    assert_eq!(resp.total_tax_minor, -850);
    assert_eq!(resp.tax_by_line[0].tax_minor, -850);
}

// ── Mixed known and unknown jurisdictions ──────────────────────────────

#[tokio::test]
async fn provider_still_ok_even_with_all_unknown() {
    // Verifies that LocalTaxProvider never returns Err for jurisdiction issues;
    // it always returns Ok with warnings instead.
    let provider = LocalTaxProvider::new(empty_config());
    let req = quote_req(addr("XX", "YY"), addr("XX", "YY"), vec![line("l1", 99999)]);
    let result = provider.quote_tax(req).await;
    assert!(result.is_ok(), "should never error on unknown jurisdiction");
}
