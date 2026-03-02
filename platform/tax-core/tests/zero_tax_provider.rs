//! Integration tests — ZeroTaxProvider for exempt entities.
//!
//! Verifies the safe-default provider always returns zero tax with appropriate
//! warnings, and that the full commit/void lifecycle succeeds.

use chrono::Utc;
use tax_core::{
    TaxAddress, TaxCommitRequest, TaxLineItem, TaxProvider, TaxQuoteRequest, TaxVoidRequest,
    ZeroTaxProvider,
};

// ── Helpers ────────────────────────────────────────────────────────────

fn addr() -> TaxAddress {
    TaxAddress {
        line1: "1 Test St".into(),
        line2: None,
        city: "Testville".into(),
        state: "CA".into(),
        postal_code: "00000".into(),
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
        tenant_id: "t-exempt".into(),
        invoice_id: "inv-exempt".into(),
        customer_id: "c-exempt".into(),
        ship_to: addr(),
        ship_from: addr(),
        line_items: lines,
        currency: "usd".into(),
        invoice_date: Utc::now(),
        correlation_id: "corr-exempt".into(),
    }
}

// ── Quote tests ────────────────────────────────────────────────────────

#[tokio::test]
async fn single_line_always_zero() {
    let provider = ZeroTaxProvider;
    let resp = provider
        .quote_tax(quote_req(vec![line("l1", 10000)]))
        .await
        .unwrap();

    assert_eq!(resp.total_tax_minor, 0);
    assert_eq!(resp.tax_by_line.len(), 1);
    assert_eq!(resp.tax_by_line[0].tax_minor, 0);
    assert_eq!(resp.tax_by_line[0].rate, 0.0);
    assert_eq!(resp.tax_by_line[0].jurisdiction, "not_configured");
    assert_eq!(resp.tax_by_line[0].tax_type, "none");
}

#[tokio::test]
async fn multiple_lines_all_zero() {
    let provider = ZeroTaxProvider;
    let resp = provider
        .quote_tax(quote_req(vec![
            line("l1", 50000),
            line("l2", 30000),
            line("l3", 70000),
        ]))
        .await
        .unwrap();

    assert_eq!(resp.total_tax_minor, 0);
    assert_eq!(resp.tax_by_line.len(), 3);
    for tbl in &resp.tax_by_line {
        assert_eq!(tbl.tax_minor, 0);
        assert_eq!(tbl.rate, 0.0);
        assert_eq!(tbl.jurisdiction, "not_configured");
    }
}

#[tokio::test]
async fn line_ids_preserved_in_breakdown() {
    let provider = ZeroTaxProvider;
    let resp = provider
        .quote_tax(quote_req(vec![line("alpha", 100), line("beta", 200)]))
        .await
        .unwrap();

    assert_eq!(resp.tax_by_line[0].line_id, "alpha");
    assert_eq!(resp.tax_by_line[1].line_id, "beta");
}

#[tokio::test]
async fn warning_always_present() {
    let provider = ZeroTaxProvider;
    let resp = provider
        .quote_tax(quote_req(vec![line("l1", 10000)]))
        .await
        .unwrap();

    assert!(
        resp.warnings
            .contains(&"jurisdiction_not_configured".to_string()),
        "ZeroTaxProvider must always emit jurisdiction_not_configured warning"
    );
}

#[tokio::test]
async fn provider_quote_ref_includes_invoice_id() {
    let provider = ZeroTaxProvider;
    let resp = provider
        .quote_tax(quote_req(vec![line("l1", 100)]))
        .await
        .unwrap();

    assert!(resp.provider_quote_ref.starts_with("zero-tax-"));
    assert!(resp.provider_quote_ref.contains("inv-exempt"));
}

#[tokio::test]
async fn quoted_at_is_recent() {
    let before = Utc::now();
    let provider = ZeroTaxProvider;
    let resp = provider
        .quote_tax(quote_req(vec![line("l1", 100)]))
        .await
        .unwrap();
    let after = Utc::now();

    assert!(resp.quoted_at >= before);
    assert!(resp.quoted_at <= after);
}

#[tokio::test]
async fn expires_at_is_none() {
    let provider = ZeroTaxProvider;
    let resp = provider
        .quote_tax(quote_req(vec![line("l1", 100)]))
        .await
        .unwrap();
    assert!(resp.expires_at.is_none());
}

// ── Large amount still zero ────────────────────────────────────────────

#[tokio::test]
async fn large_amount_still_zero() {
    let provider = ZeroTaxProvider;
    // $1,000,000 = 100_000_000 minor units
    let resp = provider
        .quote_tax(quote_req(vec![line("l1", 100_000_000)]))
        .await
        .unwrap();
    assert_eq!(resp.total_tax_minor, 0);
}

// ── Commit lifecycle ───────────────────────────────────────────────────

#[tokio::test]
async fn commit_returns_reference_with_invoice_id() {
    let provider = ZeroTaxProvider;
    let resp = provider
        .commit_tax(TaxCommitRequest {
            tenant_id: "t-exempt".into(),
            invoice_id: "inv-99".into(),
            provider_quote_ref: "zero-tax-inv-99".into(),
            correlation_id: "corr-1".into(),
        })
        .await
        .unwrap();

    assert!(resp.provider_commit_ref.starts_with("zero-commit-"));
    assert!(resp.provider_commit_ref.contains("inv-99"));
}

// ── Void lifecycle ─────────────────────────────────────────────────────

#[tokio::test]
async fn void_returns_success() {
    let provider = ZeroTaxProvider;
    let resp = provider
        .void_tax(TaxVoidRequest {
            tenant_id: "t-exempt".into(),
            invoice_id: "inv-99".into(),
            provider_commit_ref: "zero-commit-inv-99".into(),
            void_reason: "cancelled".into(),
            correlation_id: "corr-1".into(),
        })
        .await
        .unwrap();

    assert!(resp.voided);
}

// ── Full quote → commit → void lifecycle ───────────────────────────────

#[tokio::test]
async fn full_lifecycle_quote_commit_void() {
    let provider = ZeroTaxProvider;

    // 1. Quote
    let quote_resp = provider
        .quote_tax(quote_req(vec![line("l1", 50000)]))
        .await
        .unwrap();
    assert_eq!(quote_resp.total_tax_minor, 0);

    // 2. Commit
    let commit_resp = provider
        .commit_tax(TaxCommitRequest {
            tenant_id: "t-exempt".into(),
            invoice_id: "inv-exempt".into(),
            provider_quote_ref: quote_resp.provider_quote_ref,
            correlation_id: "corr-lc".into(),
        })
        .await
        .unwrap();
    assert!(commit_resp.provider_commit_ref.starts_with("zero-commit-"));

    // 3. Void
    let void_resp = provider
        .void_tax(TaxVoidRequest {
            tenant_id: "t-exempt".into(),
            invoice_id: "inv-exempt".into(),
            provider_commit_ref: commit_resp.provider_commit_ref,
            void_reason: "full refund".into(),
            correlation_id: "corr-lc".into(),
        })
        .await
        .unwrap();
    assert!(void_resp.voided);
}
