//! Avalara AvaTax integration tests.
//!
//! All tests are `#[ignore]` by default — they call the real Avalara sandbox.
//! Run with:
//!
//! ```bash
//! AVALARA_ACCOUNT_ID=... AVALARA_LICENSE_KEY=... AVALARA_COMPANY_CODE=... \
//!   ./scripts/cargo-slot.sh test -p ar-rs -- avalara_provider -- --ignored
//! ```
//!
//! No HTTP mocking. No stubs. Tests hit the live sandbox.

use ar_rs::tax::{
    AvalaraConfig, AvalaraProvider, TaxAddress, TaxCommitRequest, TaxLineItem, TaxProvider,
    TaxProviderError, TaxQuoteRequest, TaxVoidRequest,
};
use chrono::Utc;
use uuid::Uuid;

// ============================================================================
// Helpers
// ============================================================================

fn sandbox_config() -> AvalaraConfig {
    AvalaraConfig {
        account_id: std::env::var("AVALARA_ACCOUNT_ID").expect("AVALARA_ACCOUNT_ID required"),
        license_key: std::env::var("AVALARA_LICENSE_KEY").expect("AVALARA_LICENSE_KEY required"),
        company_code: std::env::var("AVALARA_COMPANY_CODE")
            .unwrap_or_else(|_| "DEFAULT".to_string()),
        base_url: std::env::var("AVALARA_BASE_URL")
            .unwrap_or_else(|_| "https://sandbox-rest.avatax.com".to_string()),
        timeout_secs: 30,
    }
}

fn ca_ship_to() -> TaxAddress {
    TaxAddress {
        line1: "100 Main St".to_string(),
        line2: None,
        city: "Los Angeles".to_string(),
        state: "CA".to_string(),
        postal_code: "90001".to_string(),
        country: "US".to_string(),
    }
}

fn origin_address() -> TaxAddress {
    TaxAddress {
        line1: "123 Commerce Blvd".to_string(),
        line2: None,
        city: "Austin".to_string(),
        state: "TX".to_string(),
        postal_code: "78701".to_string(),
        country: "US".to_string(),
    }
}

fn saas_line(line_id: &str) -> TaxLineItem {
    TaxLineItem {
        line_id: line_id.to_string(),
        description: "SaaS subscription".to_string(),
        amount_minor: 10000, // $100.00
        currency: "usd".to_string(),
        tax_code: Some("SW050000".to_string()),
        quantity: 1.0,
    }
}

fn quote_request(invoice_id: &str) -> TaxQuoteRequest {
    TaxQuoteRequest {
        tenant_id: "test-tenant".to_string(),
        invoice_id: invoice_id.to_string(),
        customer_id: "test-customer".to_string(),
        ship_to: ca_ship_to(),
        ship_from: origin_address(),
        line_items: vec![saas_line("line-1")],
        currency: "usd".to_string(),
        invoice_date: Utc::now(),
        correlation_id: Uuid::new_v4().to_string(),
    }
}

// ============================================================================
// Tests
// ============================================================================

/// Quote a $100 SaaS sale to a CA address — must return tax > 0 with a CA
/// state jurisdiction line.
#[tokio::test]
#[ignore]
async fn avalara_provider_quote_tax_california_sale_returns_expected_rate() {
    let provider = AvalaraProvider::new(sandbox_config());
    let invoice_id = format!("test-quote-ca-{}", Uuid::new_v4());
    let req = quote_request(&invoice_id);

    let resp = provider.quote_tax(req).await.expect("quote_tax failed");

    assert!(
        resp.total_tax_minor > 0,
        "Expected positive tax for CA sale, got {}",
        resp.total_tax_minor
    );
    assert!(
        !resp.tax_by_line.is_empty(),
        "Expected at least one tax_by_line entry"
    );

    let has_ca_line = resp
        .tax_by_line
        .iter()
        .any(|l| l.jurisdiction.to_uppercase().contains("CA"));
    assert!(
        has_ca_line,
        "Expected a CA jurisdiction line in tax_by_line, got: {:?}",
        resp.tax_by_line
            .iter()
            .map(|l| &l.jurisdiction)
            .collect::<Vec<_>>()
    );
    assert!(
        !resp.provider_quote_ref.is_empty(),
        "provider_quote_ref must not be empty"
    );
}

/// Commit the same invoice twice — Avalara is idempotent on document code,
/// so the second commit must return the same provider_commit_ref.
#[tokio::test]
#[ignore]
async fn avalara_provider_commit_tax_idempotent_on_replay() {
    let provider = AvalaraProvider::new(sandbox_config());
    let invoice_id = format!("test-commit-idem-{}", Uuid::new_v4());

    // First: quote to get the document code registered with Avalara
    let quote_resp = provider
        .quote_tax(quote_request(&invoice_id))
        .await
        .expect("quote_tax failed");

    let commit_req = TaxCommitRequest {
        tenant_id: "test-tenant".to_string(),
        invoice_id: invoice_id.clone(),
        provider_quote_ref: quote_resp.provider_quote_ref.clone(),
        correlation_id: Uuid::new_v4().to_string(),
    };

    let first = provider
        .commit_tax(commit_req.clone())
        .await
        .expect("first commit_tax failed");

    let second = provider
        .commit_tax(commit_req)
        .await
        .expect("second commit_tax failed");

    assert_eq!(
        first.provider_commit_ref, second.provider_commit_ref,
        "Idempotent commit must return the same provider_commit_ref"
    );
}

/// Quote → commit → void. The void must succeed with voided=true.
#[tokio::test]
#[ignore]
async fn avalara_provider_void_tax_marks_committed_transaction_voided() {
    let provider = AvalaraProvider::new(sandbox_config());
    let invoice_id = format!("test-void-{}", Uuid::new_v4());

    let quote_resp = provider
        .quote_tax(quote_request(&invoice_id))
        .await
        .expect("quote_tax failed");

    let commit_resp = provider
        .commit_tax(TaxCommitRequest {
            tenant_id: "test-tenant".to_string(),
            invoice_id: invoice_id.clone(),
            provider_quote_ref: quote_resp.provider_quote_ref,
            correlation_id: Uuid::new_v4().to_string(),
        })
        .await
        .expect("commit_tax failed");

    let void_resp = provider
        .void_tax(TaxVoidRequest {
            tenant_id: "test-tenant".to_string(),
            invoice_id: invoice_id.clone(),
            provider_commit_ref: commit_resp.provider_commit_ref,
            void_reason: "invoice_cancelled".to_string(),
            correlation_id: Uuid::new_v4().to_string(),
        })
        .await
        .expect("void_tax failed");

    assert!(void_resp.voided, "Expected voided=true");
}

/// Void with a bogus transaction code — must return ProviderError (not Unavailable).
/// Avalara returns 4xx for unknown transactions, which is a permanent error.
#[tokio::test]
#[ignore]
async fn avalara_provider_void_tax_unknown_transaction_returns_provider_error() {
    let provider = AvalaraProvider::new(sandbox_config());

    let err = provider
        .void_tax(TaxVoidRequest {
            tenant_id: "test-tenant".to_string(),
            invoice_id: "bogus-invoice".to_string(),
            provider_commit_ref: format!("nonexistent-{}", Uuid::new_v4()),
            void_reason: "test".to_string(),
            correlation_id: Uuid::new_v4().to_string(),
        })
        .await
        .expect_err("Expected error for unknown transaction");

    assert!(
        matches!(err, TaxProviderError::Provider(_)),
        "Expected TaxProviderError::Provider for unknown transaction, got: {:?}",
        err
    );
}

/// Point base_url at a routable-but-silent host with a 1-second timeout.
/// Must return Unavailable (transient, safe for retry).
#[tokio::test]
#[ignore]
async fn avalara_provider_quote_tax_network_timeout_returns_unavailable() {
    let cfg = AvalaraConfig {
        account_id: "dummy".to_string(),
        license_key: "dummy".to_string(),
        company_code: "DEFAULT".to_string(),
        base_url: "http://10.255.255.1:1".to_string(),
        timeout_secs: 1,
    };
    let provider = AvalaraProvider::new(cfg);

    let err = provider
        .quote_tax(TaxQuoteRequest {
            tenant_id: "t".to_string(),
            invoice_id: "inv-timeout-test".to_string(),
            customer_id: "c".to_string(),
            ship_to: ca_ship_to(),
            ship_from: origin_address(),
            line_items: vec![saas_line("line-1")],
            currency: "usd".to_string(),
            invoice_date: Utc::now(),
            correlation_id: "corr-timeout".to_string(),
        })
        .await
        .expect_err("Expected timeout error");

    assert!(
        matches!(err, TaxProviderError::Unavailable(_)),
        "Expected TaxProviderError::Unavailable for network timeout, got: {:?}",
        err
    );
}
