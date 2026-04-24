//! Tax and FX settlement event contracts:
//! tax.quoted, tax.committed, tax.voided, ar.invoice_settled_fx

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{AR_EVENT_SCHEMA_VERSION, MUTATION_CLASS_DATA_MUTATION, MUTATION_CLASS_REVERSAL};
use crate::events::envelope::{create_ar_envelope, EventEnvelope};

// ============================================================================
// Event Type Constants
// ============================================================================

/// Tenant tax calculation source or provider was changed by an admin
pub const EVENT_TYPE_TAX_CONFIG_CHANGED: &str = "ar.tax_config_changed";

/// Tax was calculated for an invoice draft (pre-commit, reversible)
pub const EVENT_TYPE_TAX_QUOTED: &str = "tax.quoted";

/// Tax was committed when invoice was finalized (legally due)
pub const EVENT_TYPE_TAX_COMMITTED: &str = "tax.committed";

/// A committed tax transaction was voided (refund, write-off, or cancellation)
pub const EVENT_TYPE_TAX_VOIDED: &str = "tax.voided";

/// A foreign-currency receivable was settled at a different rate than recognition
pub const EVENT_TYPE_INVOICE_SETTLED_FX: &str = "ar.invoice_settled_fx";

// ============================================================================
// Payload: tax.quoted
// ============================================================================

/// Per-line tax detail included in tax.quoted payload
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaxLineDetail {
    pub line_id: String,
    /// Tax amount for this line in minor currency units
    pub tax_minor: i64,
    /// Effective rate (0.0–1.0)
    pub rate: f64,
    pub jurisdiction: String,
    pub tax_type: String,
}

/// Payload for tax.quoted
///
/// Emitted after a successful tax quote for an invoice draft.
/// The provider_quote_ref may be used to commit or void.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaxQuotedPayload {
    pub tenant_id: String,
    pub invoice_id: String,
    pub customer_id: String,
    /// Total tax across all lines in minor currency units
    pub total_tax_minor: i64,
    pub currency: String,
    /// Per-line tax breakdown (for auditability)
    pub tax_by_line: Vec<TaxLineDetail>,
    /// Provider-assigned quote reference (used to commit/void)
    pub provider_quote_ref: String,
    /// Tax provider used (e.g. "avalara", "taxjar", "local")
    pub provider: String,
    pub quoted_at: DateTime<Utc>,
}

/// Build an envelope for tax.quoted
///
/// mutation_class: DATA_MUTATION (creates a tax quote record)
pub fn build_tax_quoted_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: TaxQuotedPayload,
) -> EventEnvelope<TaxQuotedPayload> {
    create_ar_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_TAX_QUOTED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(AR_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Payload: tax.committed
// ============================================================================

/// Payload for tax.committed
///
/// Emitted when a tax transaction is committed at invoice finalization.
/// From this point, the tax liability is legally recorded with the provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaxCommittedPayload {
    pub tenant_id: String,
    pub invoice_id: String,
    pub customer_id: String,
    /// Total committed tax in minor currency units
    pub total_tax_minor: i64,
    pub currency: String,
    /// Quote reference that was committed
    pub provider_quote_ref: String,
    /// Provider-assigned reference for the committed transaction
    pub provider_commit_ref: String,
    pub provider: String,
    pub committed_at: DateTime<Utc>,
}

/// Build an envelope for tax.committed
///
/// mutation_class: DATA_MUTATION (records committed tax liability)
pub fn build_tax_committed_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: TaxCommittedPayload,
) -> EventEnvelope<TaxCommittedPayload> {
    create_ar_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_TAX_COMMITTED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(AR_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Payload: tax.voided
// ============================================================================

/// Payload for tax.voided
///
/// Emitted when a committed tax transaction is voided.
/// Triggers reversal of tax liability (refund, write-off, cancellation).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaxVoidedPayload {
    pub tenant_id: String,
    pub invoice_id: String,
    pub customer_id: String,
    /// Total voided tax in minor currency units
    pub total_tax_minor: i64,
    pub currency: String,
    /// Commit reference that was voided
    pub provider_commit_ref: String,
    pub provider: String,
    /// Reason for void (e.g. "invoice_cancelled", "write_off", "full_refund")
    pub void_reason: String,
    pub voided_at: DateTime<Utc>,
}

/// Build an envelope for tax.voided
///
/// mutation_class: REVERSAL (compensates for the committed tax liability)
pub fn build_tax_voided_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: TaxVoidedPayload,
) -> EventEnvelope<TaxVoidedPayload> {
    create_ar_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_TAX_VOIDED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_REVERSAL.to_string(),
        payload,
    )
    .with_schema_version(AR_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Payload: ar.tax_config_changed (bd-kkhf4)
// ============================================================================

/// Payload for ar.tax_config_changed
///
/// Emitted when a tenant admin changes the tax calculation source or provider.
/// Subscribers (reconciliation worker, sync adapter) use this to open a new
/// diff window at `changed_at`, ensuring in-flight invoices are not affected.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaxConfigChangedPayload {
    pub tenant_id: String,
    /// "platform" | "external_accounting_software"
    pub tax_calculation_source: String,
    /// "local" | "zero" | "avalara"
    pub provider_name: String,
    /// New config_version after this mutation
    pub config_version: i64,
    pub updated_by: String,
    pub changed_at: DateTime<Utc>,
}

/// Build an envelope for ar.tax_config_changed
///
/// mutation_class: DATA_MUTATION (records config state change)
pub fn build_tax_config_changed_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: TaxConfigChangedPayload,
) -> EventEnvelope<TaxConfigChangedPayload> {
    create_ar_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_TAX_CONFIG_CHANGED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(AR_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Payload: ar.invoice_settled_fx (Phase 23a)
// ============================================================================

/// Payload for ar.invoice_settled_fx
///
/// Emitted when a foreign-currency receivable is settled at a different FX rate
/// than the rate used at invoice recognition. Carries both the transaction-currency
/// amount and the reporting-currency amounts at recognition and settlement rates,
/// plus rate references for audit trail.
///
/// The GL consumer uses this to post a balanced realized FX gain/loss journal entry.
/// No entry is posted when recognition and settlement amounts are equal (same rate).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvoiceSettledFxPayload {
    pub tenant_id: String,
    pub invoice_id: String,
    pub customer_id: String,
    /// Transaction (foreign) currency code (ISO 4217)
    pub txn_currency: String,
    /// Transaction amount in minor units (the invoiced amount in foreign currency)
    pub txn_amount_minor: i64,
    /// Reporting (functional) currency code (ISO 4217)
    pub rpt_currency: String,
    /// Reporting-currency amount at invoice recognition rate (minor units)
    pub recognition_rpt_amount_minor: i64,
    /// UUID of the FX rate snapshot used at recognition
    pub recognition_rate_id: Uuid,
    /// The recognition FX rate (1 txn = rate rpt)
    pub recognition_rate: f64,
    /// Reporting-currency amount at settlement rate (minor units)
    pub settlement_rpt_amount_minor: i64,
    /// UUID of the FX rate snapshot used at settlement
    pub settlement_rate_id: Uuid,
    /// The settlement FX rate (1 txn = rate rpt)
    pub settlement_rate: f64,
    /// Realized gain/loss in reporting-currency minor units (positive = gain, negative = loss)
    pub realized_gain_loss_minor: i64,
    pub settled_at: DateTime<Utc>,
}

/// Build an envelope for ar.invoice_settled_fx
///
/// mutation_class: DATA_MUTATION (posts FX gain/loss adjustment)
pub fn build_invoice_settled_fx_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: InvoiceSettledFxPayload,
) -> EventEnvelope<InvoiceSettledFxPayload> {
    create_ar_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_INVOICE_SETTLED_FX.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(AR_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn tax_quoted_envelope_has_data_mutation_class() {
        let payload = TaxQuotedPayload {
            tenant_id: "tenant-1".to_string(),
            invoice_id: "inv-1".to_string(),
            customer_id: "cust-1".to_string(),
            total_tax_minor: 850,
            currency: "usd".to_string(),
            tax_by_line: vec![TaxLineDetail {
                line_id: "line-1".to_string(),
                tax_minor: 850,
                rate: 0.085,
                jurisdiction: "California".to_string(),
                tax_type: "sales_tax".to_string(),
            }],
            provider_quote_ref: "quote-abc".to_string(),
            provider: "local".to_string(),
            quoted_at: Utc::now(),
        };
        let envelope = build_tax_quoted_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-1".to_string(),
            None,
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_TAX_QUOTED);
        assert_eq!(
            envelope.mutation_class.as_deref(),
            Some(MUTATION_CLASS_DATA_MUTATION)
        );
        assert_eq!(envelope.schema_version, AR_EVENT_SCHEMA_VERSION);
        assert_eq!(envelope.source_module, "ar");
    }

    #[test]
    fn tax_committed_envelope_has_data_mutation_class() {
        let payload = TaxCommittedPayload {
            tenant_id: "tenant-1".to_string(),
            invoice_id: "inv-1".to_string(),
            customer_id: "cust-1".to_string(),
            total_tax_minor: 850,
            currency: "usd".to_string(),
            provider_quote_ref: "quote-abc".to_string(),
            provider_commit_ref: "commit-xyz".to_string(),
            provider: "local".to_string(),
            committed_at: Utc::now(),
        };
        let envelope = build_tax_committed_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-1".to_string(),
            Some("cause-finalize".to_string()),
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_TAX_COMMITTED);
        assert_eq!(
            envelope.mutation_class.as_deref(),
            Some(MUTATION_CLASS_DATA_MUTATION)
        );
        assert_eq!(envelope.causation_id.as_deref(), Some("cause-finalize"));
        assert_eq!(envelope.schema_version, AR_EVENT_SCHEMA_VERSION);
    }

    #[test]
    fn tax_voided_envelope_has_reversal_class() {
        let payload = TaxVoidedPayload {
            tenant_id: "tenant-1".to_string(),
            invoice_id: "inv-1".to_string(),
            customer_id: "cust-1".to_string(),
            total_tax_minor: 850,
            currency: "usd".to_string(),
            provider_commit_ref: "commit-xyz".to_string(),
            provider: "local".to_string(),
            void_reason: "invoice_cancelled".to_string(),
            voided_at: Utc::now(),
        };
        let envelope = build_tax_voided_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-1".to_string(),
            None,
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_TAX_VOIDED);
        // Void is a REVERSAL of committed tax
        assert_eq!(
            envelope.mutation_class.as_deref(),
            Some(MUTATION_CLASS_REVERSAL)
        );
        assert_eq!(envelope.schema_version, AR_EVENT_SCHEMA_VERSION);
    }

    #[test]
    fn tax_event_type_constants_use_tax_prefix() {
        assert!(EVENT_TYPE_TAX_QUOTED.starts_with("tax."));
        assert!(EVENT_TYPE_TAX_COMMITTED.starts_with("tax."));
        assert!(EVENT_TYPE_TAX_VOIDED.starts_with("tax."));
    }

    #[test]
    fn tax_line_detail_serializes_correctly() -> Result<(), serde_json::Error> {
        let detail = TaxLineDetail {
            line_id: "line-1".to_string(),
            tax_minor: 500,
            rate: 0.05,
            jurisdiction: "New York".to_string(),
            tax_type: "sales_tax".to_string(),
        };
        let json = serde_json::to_string(&detail)?;
        assert!(json.contains("tax_minor"));
        assert!(json.contains("jurisdiction"));
        assert!(json.contains("New York"));
        Ok(())
    }

    #[test]
    fn invoice_settled_fx_envelope_has_data_mutation_class() {
        let payload = InvoiceSettledFxPayload {
            tenant_id: "tenant-1".to_string(),
            invoice_id: "inv-fx-1".to_string(),
            customer_id: "cust-1".to_string(),
            txn_currency: "EUR".to_string(),
            txn_amount_minor: 100000,
            rpt_currency: "USD".to_string(),
            recognition_rpt_amount_minor: 110000,
            recognition_rate_id: Uuid::new_v4(),
            recognition_rate: 1.10,
            settlement_rpt_amount_minor: 112000,
            settlement_rate_id: Uuid::new_v4(),
            settlement_rate: 1.12,
            realized_gain_loss_minor: 2000,
            settled_at: Utc::now(),
        };
        let envelope = build_invoice_settled_fx_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-1".to_string(),
            None,
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_INVOICE_SETTLED_FX);
        assert_eq!(
            envelope.mutation_class.as_deref(),
            Some(MUTATION_CLASS_DATA_MUTATION)
        );
        assert_eq!(envelope.schema_version, AR_EVENT_SCHEMA_VERSION);
        assert_eq!(envelope.source_module, "ar");
    }

    #[test]
    fn invoice_settled_fx_event_type_uses_ar_prefix() {
        assert!(EVENT_TYPE_INVOICE_SETTLED_FX.starts_with("ar."));
    }

    #[test]
    fn invoice_settled_fx_payload_serializes_correctly() -> Result<(), serde_json::Error> {
        let payload = InvoiceSettledFxPayload {
            tenant_id: "tenant-1".to_string(),
            invoice_id: "inv-fx-2".to_string(),
            customer_id: "cust-1".to_string(),
            txn_currency: "GBP".to_string(),
            txn_amount_minor: 50000,
            rpt_currency: "USD".to_string(),
            recognition_rpt_amount_minor: 63000,
            recognition_rate_id: Uuid::new_v4(),
            recognition_rate: 1.26,
            settlement_rpt_amount_minor: 62500,
            settlement_rate_id: Uuid::new_v4(),
            settlement_rate: 1.25,
            realized_gain_loss_minor: -500,
            settled_at: Utc::now(),
        };
        let json = serde_json::to_string(&payload)?;
        assert!(json.contains("txn_currency"));
        assert!(json.contains("rpt_currency"));
        assert!(json.contains("recognition_rate_id"));
        assert!(json.contains("settlement_rate_id"));
        assert!(json.contains("realized_gain_loss_minor"));
        Ok(())
    }
}
