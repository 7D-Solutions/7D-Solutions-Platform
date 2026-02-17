//! GL FX event contracts (Phase 23a)
//!
//! Defines the canonical event contracts for GL's foreign exchange events:
//! - fx.rate_updated           (an FX rate was published or updated)
//! - gl.fx_revaluation_posted  (unrealized FX gain/loss posted to GL)
//! - gl.fx_realized_posted     (realized FX gain/loss posted to GL)

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::events::envelope::{create_gl_envelope, EventEnvelope};

use super::MUTATION_CLASS_DATA_MUTATION;

// ============================================================================
// FX Event Type Constants
// ============================================================================

/// An FX rate was published or updated for a currency pair.
pub const EVENT_TYPE_FX_RATE_UPDATED: &str = "fx.rate_updated";

/// An unrealized FX gain/loss revaluation was posted to the GL.
pub const EVENT_TYPE_FX_REVALUATION_POSTED: &str = "gl.fx_revaluation_posted";

/// A realized FX gain/loss was posted to the GL (settlement crystallized).
pub const EVENT_TYPE_FX_REALIZED_POSTED: &str = "gl.fx_realized_posted";

// ============================================================================
// FX Schema Version
// ============================================================================

/// Schema version for all FX event payloads (v1)
pub const GL_FX_SCHEMA_VERSION: &str = "1.0.0";

// ============================================================================
// Payload: fx.rate_updated
// ============================================================================

/// Payload for fx.rate_updated — exchange rate created or updated for a currency pair.
/// Rate: 1 `base_currency` = `rate` `quote_currency`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FxRateUpdatedPayload {
    pub rate_id: Uuid,
    /// ISO 4217 base currency (the "1 unit of" side)
    pub base_currency: String,
    /// ISO 4217 quote currency (the "rate units of" side)
    pub quote_currency: String,
    /// Exchange rate: 1 base = rate quote
    pub rate: f64,
    /// Inverse: 1 quote = inverse_rate base
    pub inverse_rate: f64,
    pub effective_at: DateTime<Utc>,
    /// Rate source (e.g. "ecb", "openexchangerates", "manual")
    pub source: String,
}

/// Build an envelope for fx.rate_updated
///
/// mutation_class: DATA_MUTATION (creates/updates an FX rate record)
pub fn build_fx_rate_updated_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: FxRateUpdatedPayload,
) -> EventEnvelope<FxRateUpdatedPayload> {
    create_gl_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_FX_RATE_UPDATED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(GL_FX_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Payload: gl.fx_revaluation_posted
// ============================================================================

/// Payload for gl.fx_revaluation_posted — unrealized FX gain/loss at period end.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FxRevaluationPostedPayload {
    pub revaluation_id: Uuid,
    pub tenant_id: String,
    /// Accounting period (YYYY-MM)
    pub period: String,
    /// Foreign (transaction) currency being revalued
    pub transaction_currency: String,
    /// Tenant's reporting (functional) currency
    pub reporting_currency: String,
    /// Rate used: 1 transaction = rate reporting
    pub rate_used: f64,
    /// Original balance in transaction currency (minor units)
    pub original_amount_minor: i64,
    /// Revalued balance in reporting currency (minor units)
    pub revalued_amount_minor: i64,
    /// Unrealized gain (+) or loss (−) in reporting currency minor units
    pub unrealized_gain_loss_minor: i64,
    pub gain_loss_account: String,
    /// GL account for the balance being revalued
    pub balance_account: String,
    pub journal_entry_id: Option<Uuid>,
    /// Date revaluation performed (YYYY-MM-DD)
    pub revaluation_date: String,
    /// Whether auto-reversed next period
    pub auto_reverse: bool,
}

/// Build an envelope for gl.fx_revaluation_posted
///
/// mutation_class: DATA_MUTATION (posts unrealized FX gain/loss journal entry)
pub fn build_fx_revaluation_posted_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: FxRevaluationPostedPayload,
) -> EventEnvelope<FxRevaluationPostedPayload> {
    create_gl_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_FX_REVALUATION_POSTED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(GL_FX_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Payload: gl.fx_realized_posted
// ============================================================================

/// Payload for gl.fx_realized_posted — realized FX gain/loss from settlement.
/// Never reversed (represents actual economic gain/loss).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FxRealizedPostedPayload {
    pub realized_id: Uuid,
    pub tenant_id: String,
    /// Source transaction (e.g. invoice_id or payment_id)
    pub source_transaction_id: Uuid,
    /// Source type (e.g. "invoice_payment", "ar_settlement")
    pub source_transaction_type: String,
    pub transaction_currency: String,
    pub reporting_currency: String,
    /// Original booking rate
    pub booking_rate: f64,
    /// Settlement rate
    pub settlement_rate: f64,
    /// Transaction amount in transaction currency (minor units)
    pub transaction_amount_minor: i64,
    /// Realized gain (+) or loss (−) in reporting currency minor units
    pub realized_gain_loss_minor: i64,
    pub gain_loss_account: String,
    pub journal_entry_id: Option<Uuid>,
    /// Settlement date (YYYY-MM-DD)
    pub settlement_date: String,
}

/// Build an envelope for gl.fx_realized_posted
///
/// mutation_class: DATA_MUTATION (posts realized FX gain/loss journal entry)
pub fn build_fx_realized_posted_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: FxRealizedPostedPayload,
) -> EventEnvelope<FxRealizedPostedPayload> {
    create_gl_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_FX_REALIZED_POSTED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(GL_FX_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    // ─── FX Event Type Constants ────────────────────────────────────────────

    #[test]
    fn fx_event_type_constants_use_correct_prefix() {
        assert!(EVENT_TYPE_FX_RATE_UPDATED.starts_with("fx."));
        assert!(EVENT_TYPE_FX_REVALUATION_POSTED.starts_with("gl."));
        assert!(EVENT_TYPE_FX_REALIZED_POSTED.starts_with("gl."));
    }

    #[test]
    fn fx_schema_version_is_stable() {
        assert_eq!(GL_FX_SCHEMA_VERSION, "1.0.0");
    }

    // ─── fx.rate_updated ────────────────────────────────────────────────────

    fn sample_fx_rate_updated() -> FxRateUpdatedPayload {
        FxRateUpdatedPayload {
            rate_id: Uuid::new_v4(),
            base_currency: "EUR".to_string(),
            quote_currency: "USD".to_string(),
            rate: 1.085,
            inverse_rate: 0.921658986,
            effective_at: Utc::now(),
            source: "ecb".to_string(),
        }
    }

    #[test]
    fn fx_rate_updated_envelope_has_correct_type_and_class() {
        let envelope = build_fx_rate_updated_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-fx-1".to_string(),
            None,
            sample_fx_rate_updated(),
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_FX_RATE_UPDATED);
        assert_eq!(
            envelope.mutation_class.as_deref(),
            Some(MUTATION_CLASS_DATA_MUTATION)
        );
        assert_eq!(envelope.schema_version, GL_FX_SCHEMA_VERSION);
        assert_eq!(envelope.source_module, "gl");
    }

    #[test]
    fn fx_rate_updated_payload_serializes_correctly() {
        let payload = sample_fx_rate_updated();
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("base_currency"));
        assert!(json.contains("quote_currency"));
        assert!(json.contains("rate"));
        assert!(json.contains("inverse_rate"));
        assert!(json.contains("source"));
        assert!(json.contains("\"EUR\""));
        assert!(json.contains("\"USD\""));
    }

    #[test]
    fn fx_rate_updated_roundtrips() {
        let payload = sample_fx_rate_updated();
        let json = serde_json::to_string(&payload).unwrap();
        let roundtrip: FxRateUpdatedPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(roundtrip.base_currency, "EUR");
        assert_eq!(roundtrip.quote_currency, "USD");
        assert!((roundtrip.rate - 1.085).abs() < f64::EPSILON);
    }

    // ─── gl.fx_revaluation_posted ───────────────────────────────────────────

    fn sample_fx_revaluation() -> FxRevaluationPostedPayload {
        FxRevaluationPostedPayload {
            revaluation_id: Uuid::new_v4(),
            tenant_id: "tenant-1".to_string(),
            period: "2026-01".to_string(),
            transaction_currency: "EUR".to_string(),
            reporting_currency: "USD".to_string(),
            rate_used: 1.085,
            original_amount_minor: 100000, // 1000.00 EUR
            revalued_amount_minor: 108500, // 1085.00 USD
            unrealized_gain_loss_minor: 500, // 5.00 USD gain
            gain_loss_account: "UNREALIZED_FX_GAIN_LOSS".to_string(),
            balance_account: "AR_EUR".to_string(),
            journal_entry_id: Some(Uuid::new_v4()),
            revaluation_date: "2026-01-31".to_string(),
            auto_reverse: true,
        }
    }

    #[test]
    fn fx_revaluation_envelope_has_correct_type_and_class() {
        let envelope = build_fx_revaluation_posted_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-reval-1".to_string(),
            None,
            sample_fx_revaluation(),
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_FX_REVALUATION_POSTED);
        assert_eq!(
            envelope.mutation_class.as_deref(),
            Some(MUTATION_CLASS_DATA_MUTATION)
        );
        assert_eq!(envelope.schema_version, GL_FX_SCHEMA_VERSION);
        assert_eq!(envelope.source_module, "gl");
    }

    #[test]
    fn fx_revaluation_payload_carries_currency_pair() {
        let payload = sample_fx_revaluation();
        assert_eq!(payload.transaction_currency, "EUR");
        assert_eq!(payload.reporting_currency, "USD");
    }

    #[test]
    fn fx_revaluation_payload_serializes_correctly() {
        let payload = sample_fx_revaluation();
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("transaction_currency"));
        assert!(json.contains("reporting_currency"));
        assert!(json.contains("unrealized_gain_loss_minor"));
        assert!(json.contains("gain_loss_account"));
        assert!(json.contains("auto_reverse"));
    }

    #[test]
    fn fx_revaluation_envelope_carries_causation_id() {
        let envelope = build_fx_revaluation_posted_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-reval-1".to_string(),
            Some("cause-period-close".to_string()),
            sample_fx_revaluation(),
        );
        assert_eq!(
            envelope.causation_id.as_deref(),
            Some("cause-period-close")
        );
    }

    // ─── gl.fx_realized_posted ──────────────────────────────────────────────

    fn sample_fx_realized() -> FxRealizedPostedPayload {
        FxRealizedPostedPayload {
            realized_id: Uuid::new_v4(),
            tenant_id: "tenant-1".to_string(),
            source_transaction_id: Uuid::new_v4(),
            source_transaction_type: "invoice_payment".to_string(),
            transaction_currency: "EUR".to_string(),
            reporting_currency: "USD".to_string(),
            booking_rate: 1.08,
            settlement_rate: 1.085,
            transaction_amount_minor: 100000, // 1000.00 EUR
            realized_gain_loss_minor: 500,    // 5.00 USD gain
            gain_loss_account: "REALIZED_FX_GAIN_LOSS".to_string(),
            journal_entry_id: Some(Uuid::new_v4()),
            settlement_date: "2026-02-15".to_string(),
        }
    }

    #[test]
    fn fx_realized_envelope_has_correct_type_and_class() {
        let envelope = build_fx_realized_posted_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-real-1".to_string(),
            None,
            sample_fx_realized(),
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_FX_REALIZED_POSTED);
        assert_eq!(
            envelope.mutation_class.as_deref(),
            Some(MUTATION_CLASS_DATA_MUTATION)
        );
        assert_eq!(envelope.schema_version, GL_FX_SCHEMA_VERSION);
        assert_eq!(envelope.source_module, "gl");
    }

    #[test]
    fn fx_realized_payload_carries_rate_pair() {
        let payload = sample_fx_realized();
        assert!((payload.booking_rate - 1.08).abs() < f64::EPSILON);
        assert!((payload.settlement_rate - 1.085).abs() < f64::EPSILON);
        // Gain = (settlement - booking) * amount = (1.085 - 1.08) * 100000 = 500
        assert_eq!(payload.realized_gain_loss_minor, 500);
    }

    #[test]
    fn fx_realized_payload_serializes_correctly() {
        let payload = sample_fx_realized();
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("source_transaction_id"));
        assert!(json.contains("source_transaction_type"));
        assert!(json.contains("booking_rate"));
        assert!(json.contains("settlement_rate"));
        assert!(json.contains("realized_gain_loss_minor"));
        assert!(json.contains("settlement_date"));
    }

    #[test]
    fn fx_realized_envelope_carries_causation_id() {
        let envelope = build_fx_realized_posted_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-real-1".to_string(),
            Some("cause-payment-received".to_string()),
            sample_fx_realized(),
        );
        assert_eq!(
            envelope.causation_id.as_deref(),
            Some("cause-payment-received")
        );
    }

    // ─── Envelope enforcement: all FX events are envelope-complete ──────────

    #[test]
    fn all_fx_envelopes_pass_validation() {
        use event_bus::validate_envelope_fields;

        // fx.rate_updated
        let env1 = build_fx_rate_updated_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-1".to_string(),
            None,
            sample_fx_rate_updated(),
        );
        let json1 = serde_json::to_value(&env1).unwrap();
        assert!(
            validate_envelope_fields(&json1).is_ok(),
            "fx.rate_updated failed envelope validation: {:?}",
            validate_envelope_fields(&json1)
        );

        // gl.fx_revaluation_posted
        let env2 = build_fx_revaluation_posted_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-2".to_string(),
            None,
            sample_fx_revaluation(),
        );
        let json2 = serde_json::to_value(&env2).unwrap();
        assert!(
            validate_envelope_fields(&json2).is_ok(),
            "gl.fx_revaluation_posted failed envelope validation: {:?}",
            validate_envelope_fields(&json2)
        );

        // gl.fx_realized_posted
        let env3 = build_fx_realized_posted_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-3".to_string(),
            None,
            sample_fx_realized(),
        );
        let json3 = serde_json::to_value(&env3).unwrap();
        assert!(
            validate_envelope_fields(&json3).is_ok(),
            "gl.fx_realized_posted failed envelope validation: {:?}",
            validate_envelope_fields(&json3)
        );
    }
}
