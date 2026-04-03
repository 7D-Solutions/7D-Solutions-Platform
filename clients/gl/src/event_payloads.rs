//! Typed event payload structs for GL service events.
//!
//! Consumers can deserialize `EventEnvelope<serde_json::Value>` payloads into
//! these concrete types using `serde_json::from_value`. Each struct matches
//! the canonical schema defined in the GL module's event contracts.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ============================================================================
// Event Type Constants — FX
// ============================================================================

pub const EVENT_TYPE_FX_RATE_UPDATED: &str = "fx.rate_updated";
pub const EVENT_TYPE_FX_REVALUATION_POSTED: &str = "gl.fx_revaluation_posted";
pub const EVENT_TYPE_FX_REALIZED_POSTED: &str = "gl.fx_realized_posted";

// ============================================================================
// Event Type Constants — Accruals
// ============================================================================

pub const EVENT_TYPE_ACCRUAL_CREATED: &str = "gl.accrual_created";
pub const EVENT_TYPE_ACCRUAL_REVERSED: &str = "gl.accrual_reversed";

// ============================================================================
// Event Type Constants — Revenue Recognition
// ============================================================================

pub const EVENT_TYPE_CONTRACT_CREATED: &str = "revrec.contract_created";
pub const EVENT_TYPE_SCHEDULE_CREATED: &str = "revrec.schedule_created";
pub const EVENT_TYPE_RECOGNITION_POSTED: &str = "revrec.recognition_posted";
pub const EVENT_TYPE_CONTRACT_MODIFIED: &str = "revrec.contract_modified";

// ============================================================================
// FX Payloads
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FxRateUpdatedPayload {
    pub rate_id: Uuid,
    pub base_currency: String,
    pub quote_currency: String,
    pub rate: f64,
    pub inverse_rate: f64,
    pub effective_at: DateTime<Utc>,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FxRevaluationPostedPayload {
    pub revaluation_id: Uuid,
    pub tenant_id: String,
    pub period: String,
    pub transaction_currency: String,
    pub reporting_currency: String,
    pub rate_used: f64,
    pub original_amount_minor: i64,
    pub revalued_amount_minor: i64,
    pub unrealized_gain_loss_minor: i64,
    pub gain_loss_account: String,
    pub balance_account: String,
    pub journal_entry_id: Option<Uuid>,
    pub revaluation_date: String,
    pub auto_reverse: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FxRealizedPostedPayload {
    pub realized_id: Uuid,
    pub tenant_id: String,
    pub source_transaction_id: Uuid,
    pub source_transaction_type: String,
    pub transaction_currency: String,
    pub reporting_currency: String,
    pub booking_rate: f64,
    pub settlement_rate: f64,
    pub transaction_amount_minor: i64,
    pub realized_gain_loss_minor: i64,
    pub gain_loss_account: String,
    pub journal_entry_id: Option<Uuid>,
    pub settlement_date: String,
}

// ============================================================================
// Accrual Payloads
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CashFlowClass {
    Operating,
    Investing,
    Financing,
    NonCash,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CashFlowClassification {
    pub account_ref: String,
    pub cash_flow_class: CashFlowClass,
    pub label: String,
    pub increase_is_outflow: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReversalPolicy {
    pub auto_reverse_next_period: bool,
    pub reverse_on_date: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccrualCreatedPayload {
    pub accrual_id: Uuid,
    pub template_id: Option<Uuid>,
    pub tenant_id: String,
    pub name: String,
    pub period: String,
    pub posting_date: String,
    pub debit_account: String,
    pub credit_account: String,
    pub amount_minor: i64,
    pub currency: String,
    pub cashflow_class: CashFlowClass,
    pub reversal_policy: ReversalPolicy,
    pub journal_entry_id: Option<Uuid>,
    pub description: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccrualReversedPayload {
    pub reversal_id: Uuid,
    pub original_accrual_id: Uuid,
    pub template_id: Option<Uuid>,
    pub tenant_id: String,
    pub reversal_period: String,
    pub reversal_date: String,
    pub debit_account: String,
    pub credit_account: String,
    pub amount_minor: i64,
    pub currency: String,
    pub cashflow_class: CashFlowClass,
    pub journal_entry_id: Option<Uuid>,
    pub reason: String,
    pub reversed_at: DateTime<Utc>,
}

// ============================================================================
// Revenue Recognition Payloads
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RecognitionPattern {
    RatableOverTime { period_months: u32 },
    PointInTime,
    UsageBased {
        metric: String,
        total_contracted_quantity: f64,
        unit: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceObligation {
    pub obligation_id: Uuid,
    pub name: String,
    pub description: String,
    pub allocated_amount_minor: i64,
    pub recognition_pattern: RecognitionPattern,
    pub satisfaction_start: String,
    pub satisfaction_end: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleLine {
    pub period: String,
    pub amount_to_recognize_minor: i64,
    pub deferred_revenue_account: String,
    pub recognized_revenue_account: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModificationType {
    PriceChange,
    TermExtension,
    ObligationAdded,
    ObligationRemoved,
    Combined,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllocationChange {
    pub obligation_id: Uuid,
    pub previous_allocated_minor: i64,
    pub new_allocated_minor: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractCreatedPayload {
    pub contract_id: Uuid,
    pub tenant_id: String,
    pub customer_id: String,
    pub contract_name: String,
    pub contract_start: String,
    pub contract_end: Option<String>,
    pub total_transaction_price_minor: i64,
    pub currency: String,
    pub performance_obligations: Vec<PerformanceObligation>,
    pub external_contract_ref: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleCreatedPayload {
    pub schedule_id: Uuid,
    pub contract_id: Uuid,
    pub obligation_id: Uuid,
    pub tenant_id: String,
    pub total_to_recognize_minor: i64,
    pub currency: String,
    pub lines: Vec<ScheduleLine>,
    pub first_period: String,
    pub last_period: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecognitionPostedPayload {
    pub run_id: Uuid,
    pub contract_id: Uuid,
    pub obligation_id: Uuid,
    pub schedule_id: Uuid,
    pub tenant_id: String,
    pub period: String,
    pub posting_date: String,
    pub amount_recognized_minor: i64,
    pub currency: String,
    pub deferred_revenue_account: String,
    pub recognized_revenue_account: String,
    pub journal_entry_id: Option<Uuid>,
    pub cumulative_recognized_minor: i64,
    pub remaining_deferred_minor: i64,
    pub recognized_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractModifiedPayload {
    pub modification_id: Uuid,
    pub contract_id: Uuid,
    pub tenant_id: String,
    pub modification_type: ModificationType,
    pub effective_date: String,
    pub new_transaction_price_minor: Option<i64>,
    pub added_obligations: Vec<PerformanceObligation>,
    pub removed_obligation_ids: Vec<Uuid>,
    pub reallocated_amounts: Vec<AllocationChange>,
    pub reason: String,
    pub requires_cumulative_catchup: bool,
    pub modified_at: DateTime<Utc>,
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_type_constants_match_canonical_values() {
        assert_eq!(EVENT_TYPE_FX_RATE_UPDATED, "fx.rate_updated");
        assert_eq!(EVENT_TYPE_FX_REVALUATION_POSTED, "gl.fx_revaluation_posted");
        assert_eq!(EVENT_TYPE_FX_REALIZED_POSTED, "gl.fx_realized_posted");
        assert_eq!(EVENT_TYPE_ACCRUAL_CREATED, "gl.accrual_created");
        assert_eq!(EVENT_TYPE_ACCRUAL_REVERSED, "gl.accrual_reversed");
        assert_eq!(EVENT_TYPE_CONTRACT_CREATED, "revrec.contract_created");
        assert_eq!(EVENT_TYPE_SCHEDULE_CREATED, "revrec.schedule_created");
        assert_eq!(EVENT_TYPE_RECOGNITION_POSTED, "revrec.recognition_posted");
        assert_eq!(EVENT_TYPE_CONTRACT_MODIFIED, "revrec.contract_modified");
    }

    #[test]
    fn cashflow_class_serializes_as_snake_case() {
        assert_eq!(serde_json::to_string(&CashFlowClass::Operating).unwrap(), "\"operating\"");
        assert_eq!(serde_json::to_string(&CashFlowClass::NonCash).unwrap(), "\"non_cash\"");
    }

    #[test]
    fn recognition_pattern_ratable_roundtrips() {
        let pattern = RecognitionPattern::RatableOverTime { period_months: 12 };
        let json = serde_json::to_string(&pattern).unwrap();
        let rt: RecognitionPattern = serde_json::from_str(&json).unwrap();
        assert_eq!(rt, pattern);
    }

    #[test]
    fn modification_type_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&ModificationType::PriceChange).unwrap(),
            "\"price_change\""
        );
        assert_eq!(
            serde_json::to_string(&ModificationType::Combined).unwrap(),
            "\"combined\""
        );
    }

    #[test]
    fn fx_rate_updated_payload_roundtrips() {
        let payload = FxRateUpdatedPayload {
            rate_id: Uuid::new_v4(),
            base_currency: "EUR".to_string(),
            quote_currency: "USD".to_string(),
            rate: 1.085,
            inverse_rate: 0.9217,
            effective_at: Utc::now(),
            source: "ecb".to_string(),
        };
        let json = serde_json::to_string(&payload).unwrap();
        let _: FxRateUpdatedPayload = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn accrual_created_payload_roundtrips() {
        let payload = AccrualCreatedPayload {
            accrual_id: Uuid::new_v4(),
            template_id: None,
            tenant_id: "t".to_string(),
            name: "test".to_string(),
            period: "2026-01".to_string(),
            posting_date: "2026-01-01".to_string(),
            debit_account: "PREPAID".to_string(),
            credit_account: "CASH".to_string(),
            amount_minor: 120000,
            currency: "USD".to_string(),
            cashflow_class: CashFlowClass::Operating,
            reversal_policy: ReversalPolicy {
                auto_reverse_next_period: true,
                reverse_on_date: None,
            },
            journal_entry_id: None,
            description: "test".to_string(),
            created_at: Utc::now(),
        };
        let json = serde_json::to_string(&payload).unwrap();
        let _: AccrualCreatedPayload = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn contract_created_payload_roundtrips() {
        let payload = ContractCreatedPayload {
            contract_id: Uuid::new_v4(),
            tenant_id: "t".to_string(),
            customer_id: "c".to_string(),
            contract_name: "test".to_string(),
            contract_start: "2026-01-01".to_string(),
            contract_end: None,
            total_transaction_price_minor: 120000,
            currency: "USD".to_string(),
            performance_obligations: vec![PerformanceObligation {
                obligation_id: Uuid::new_v4(),
                name: "License".to_string(),
                description: "SaaS".to_string(),
                allocated_amount_minor: 120000,
                recognition_pattern: RecognitionPattern::RatableOverTime { period_months: 12 },
                satisfaction_start: "2026-01-01".to_string(),
                satisfaction_end: Some("2026-12-31".to_string()),
            }],
            external_contract_ref: None,
            created_at: Utc::now(),
        };
        let json = serde_json::to_string(&payload).unwrap();
        let _: ContractCreatedPayload = serde_json::from_str(&json).unwrap();
    }
}
