//! Tax data models — request/response types, value objects, and jurisdiction snapshots.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

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
// Jurisdiction resolution types (bd-360)
// ============================================================================

/// Resolved jurisdiction rule for a single line item
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedRule {
    pub jurisdiction_id: Uuid,
    pub jurisdiction_name: String,
    pub tax_type: String,
    pub rate: f64,
    pub flat_amount_minor: i64,
    pub is_exempt: bool,
    pub tax_code: Option<String>,
    pub effective_from: chrono::NaiveDate,
    pub effective_to: Option<chrono::NaiveDate>,
    pub priority: i32,
}

/// Complete resolved jurisdiction snapshot for an invoice
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JurisdictionSnapshot {
    pub jurisdiction_id: Uuid,
    pub jurisdiction_name: String,
    pub country_code: String,
    pub state_code: Option<String>,
    pub ship_to_address: TaxAddress,
    pub resolved_rules: Vec<ResolvedRule>,
    pub total_tax_minor: i64,
    pub tax_code: Option<String>,
    pub applied_rate: f64,
    pub resolution_hash: String,
    pub resolved_as_of: chrono::NaiveDate,
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

    #[test]
    fn resolved_rule_serializes() {
        let rule = ResolvedRule {
            jurisdiction_id: Uuid::new_v4(),
            jurisdiction_name: "California State Tax".to_string(),
            tax_type: "sales_tax".to_string(),
            rate: 0.085,
            flat_amount_minor: 0,
            is_exempt: false,
            tax_code: Some("SW050000".to_string()),
            effective_from: chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            effective_to: None,
            priority: 10,
        };
        let json = serde_json::to_string(&rule).unwrap();
        assert!(json.contains("jurisdiction_name"));
        assert!(json.contains("California State Tax"));
        assert!(json.contains("0.085"));
    }

    #[test]
    fn jurisdiction_snapshot_serializes() {
        let snapshot = JurisdictionSnapshot {
            jurisdiction_id: Uuid::new_v4(),
            jurisdiction_name: "California State Tax".to_string(),
            country_code: "US".to_string(),
            state_code: Some("CA".to_string()),
            ship_to_address: sample_address(),
            resolved_rules: vec![],
            total_tax_minor: 850,
            tax_code: Some("SW050000".to_string()),
            applied_rate: 0.085,
            resolution_hash: "abc123".to_string(),
            resolved_as_of: chrono::NaiveDate::from_ymd_opt(2026, 2, 17).unwrap(),
        };
        let json = serde_json::to_string(&snapshot).unwrap();
        assert!(json.contains("resolution_hash"));
        assert!(json.contains("resolved_as_of"));
        assert!(json.contains("total_tax_minor"));
    }
}
