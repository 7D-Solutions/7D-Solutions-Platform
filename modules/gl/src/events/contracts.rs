//! GL event type constants and payload structs (Phase 24b)
//!
//! Defines the canonical event contracts for GL's accrual lifecycle events:
//! - gl.accrual_created   (an accrual posting was applied to a period)
//! - gl.accrual_reversed  (an accrual was compensated by a reversal posting)
//!
//! Also defines:
//! - `CashFlowClass` enum — standard classification for the indirect cash flow statement
//! - `ReversalPolicy` struct — when/how accruals are reversed
//! - `CashFlowClassification` struct — maps account_ref → CashFlowClass
//!
//! ## Accounting semantics
//!
//! ### Accrual (gl.accrual_created)
//! An accrual records expense or revenue that has been incurred but not yet paid/received.
//! Example: DR Prepaid Insurance / CR Cash (or DR Expense / CR Accrued Liability)
//!
//! ### Reversal (gl.accrual_reversed)
//! In the following period the accrual is reversed with a compensating entry:
//! Example: DR Accrued Liability / CR Expense
//! This prevents double-counting when the actual cash transaction is posted.
//!
//! ## Idempotency
//! All events carry a caller-supplied event_id derived from the accrual_id / reversal_id.
//! Duplicate event_ids are silently skipped by `process_gl_posting_request`.
//!
//! ## Cash flow classification
//! Every accrual carries a `CashFlowClass` that determines which section of the
//! indirect cash flow statement the non-cash adjustment appears in.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::events::envelope::{create_gl_envelope, EventEnvelope};

// ============================================================================
// Event Type Constants
// ============================================================================

/// An accrual entry was created and posted to the GL (balances the books for a period)
pub const EVENT_TYPE_ACCRUAL_CREATED: &str = "gl.accrual_created";

/// A previously created accrual was reversed with a compensating journal entry
pub const EVENT_TYPE_ACCRUAL_REVERSED: &str = "gl.accrual_reversed";

// ============================================================================
// Schema Version
// ============================================================================

/// Schema version for all GL accrual event payloads (v1)
pub const GL_ACCRUAL_SCHEMA_VERSION: &str = "1.0.0";

// ============================================================================
// Mutation Classes
// ============================================================================

/// DATA_MUTATION: creates or modifies a financial record
pub const MUTATION_CLASS_DATA_MUTATION: &str = "DATA_MUTATION";

/// REVERSAL: compensates for a prior DATA_MUTATION (accrual reversal)
pub const MUTATION_CLASS_REVERSAL: &str = "REVERSAL";

// ============================================================================
// Cash Flow Classification
// ============================================================================

/// Standard indirect cash flow statement sections.
///
/// Used to classify non-cash adjustments in the operating/investing/financing
/// sections of the cash flow statement derived from posted journal entries.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CashFlowClass {
    /// Operating activities: net income adjustments, working capital changes
    /// Examples: accounts receivable, accounts payable, accrued liabilities,
    ///           depreciation/amortization, prepaid expenses
    Operating,

    /// Investing activities: long-term asset acquisitions/disposals
    /// Examples: capital expenditures, purchase of investments, proceeds from asset sales
    Investing,

    /// Financing activities: debt and equity transactions
    /// Examples: proceeds from borrowing, repayment of debt, dividends paid, equity issuance
    Financing,

    /// Non-cash item: included as a reconciling adjustment in operating activities
    /// Examples: depreciation expense, amortization of intangibles, stock-based compensation
    NonCash,
}

/// Maps a GL account reference to its cash flow classification.
///
/// This drives the indirect cash flow statement derivation:
/// - All journal lines are grouped by their account's CashFlowClass
/// - The net change per class is presented as the operating/investing/financing section
///
/// Example mappings:
/// - "AR"       → Operating (decrease in AR = cash inflow)
/// - "AP"       → Operating (increase in AP = non-cash operating adjustment)
/// - "PREPAID"  → Operating (prepaid expense amortization)
/// - "PPE"      → Investing  (capital expenditure)
/// - "DEBT"     → Financing  (debt repayment)
/// - "DEPR"     → NonCash    (depreciation add-back)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CashFlowClassification {
    /// GL account reference (e.g. "AR", "PREPAID", "PPE")
    pub account_ref: String,

    /// Which cash flow section this account maps to
    pub cash_flow_class: CashFlowClass,

    /// Human-readable label for this classification (for report headings)
    pub label: String,

    /// Whether increases in this account are inflows (+) or outflows (−)
    /// For assets: true = increase is outflow (e.g. more AR = cash not yet received)
    /// For liabilities: false = increase is inflow (e.g. more AP = cash not yet paid)
    pub increase_is_outflow: bool,
}

// ============================================================================
// Reversal Policy
// ============================================================================

/// Defines when and how an accrual entry should be reversed.
///
/// The reversal policy is locked at accrual-creation time and referenced
/// by the reversal scheduler to avoid ad-hoc reversal logic.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReversalPolicy {
    /// Auto-reverse in the next accounting period (most common for short-term accruals)
    ///
    /// When true: reversal is scheduled for the first day of the next period.
    /// When false: `reverse_on_date` must be set.
    pub auto_reverse_next_period: bool,

    /// Explicit reversal date (YYYY-MM-DD) when not auto-reversing by period.
    ///
    /// Required when `auto_reverse_next_period` is false.
    /// Ignored when `auto_reverse_next_period` is true.
    pub reverse_on_date: Option<String>,
}

// ============================================================================
// Payload: gl.accrual_created
// ============================================================================

/// Payload for gl.accrual_created
///
/// Emitted when an accrual posting is applied to the GL for a specific period.
/// The accrual creates a balanced journal entry (debit + credit = 0 net).
///
/// Idempotency: caller MUST supply a deterministic event_id derived from accrual_id.
/// Duplicate accrual_ids are silently skipped by the processed_events table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccrualCreatedPayload {
    /// Stable business key for this accrual instance (idempotency anchor)
    /// Deterministically derived from (template_id, period) or caller-assigned UUID.
    pub accrual_id: Uuid,

    /// The accrual template this instance was generated from (if template-driven)
    /// None for ad-hoc / manual accruals.
    pub template_id: Option<Uuid>,

    pub tenant_id: String,

    /// Human-readable name for the accrual (e.g. "Prepaid Insurance — Jan 2026")
    pub name: String,

    /// Accounting period this accrual belongs to (YYYY-MM)
    pub period: String,

    /// Accounting date the entry is posted (YYYY-MM-DD)
    pub posting_date: String,

    /// Account debited (e.g. "PREPAID" for prepaid expense, "EXPENSE" for accrued expense)
    pub debit_account: String,

    /// Account credited (e.g. "CASH" for prepaid, "ACCRUED_LIAB" for accrued expense)
    pub credit_account: String,

    /// Amount in minor currency units (positive, e.g. cents for USD)
    pub amount_minor: i64,

    /// ISO 4217 currency code (uppercase, e.g. "USD")
    pub currency: String,

    /// Cash flow classification for the indirect cash flow statement derivation
    pub cashflow_class: CashFlowClass,

    /// When and how this accrual should be reversed
    pub reversal_policy: ReversalPolicy,

    /// GL journal entry ID created for this accrual (populated after posting succeeds)
    pub journal_entry_id: Option<Uuid>,

    /// Optional description passed through to the journal entry memo
    pub description: String,

    pub created_at: DateTime<Utc>,
}

/// Build an envelope for gl.accrual_created
///
/// mutation_class: DATA_MUTATION (creates a new accrual + journal entry)
pub fn build_accrual_created_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: AccrualCreatedPayload,
) -> EventEnvelope<AccrualCreatedPayload> {
    create_gl_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_ACCRUAL_CREATED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(GL_ACCRUAL_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Payload: gl.accrual_reversed
// ============================================================================

/// Payload for gl.accrual_reversed
///
/// Emitted when a previously-created accrual is reversed by a compensating entry.
/// The reversal swaps debit/credit accounts relative to the original accrual,
/// effectively unwinding the accrual balance in the new period.
///
/// Idempotency: caller MUST supply a deterministic event_id derived from reversal_id.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccrualReversedPayload {
    /// Stable business key for this reversal (idempotency anchor)
    pub reversal_id: Uuid,

    /// The original accrual being reversed
    pub original_accrual_id: Uuid,

    /// The template that generated the original accrual (if template-driven)
    pub template_id: Option<Uuid>,

    pub tenant_id: String,

    /// Accounting period of the reversal (YYYY-MM) — typically the period after the accrual
    pub reversal_period: String,

    /// Accounting date of the reversal posting (YYYY-MM-DD)
    pub reversal_date: String,

    /// Account debited in the reversal (was credited in the original accrual)
    /// Example: "ACCRUED_LIAB" (reversal debits the liability to zero it out)
    pub debit_account: String,

    /// Account credited in the reversal (was debited in the original accrual)
    /// Example: "EXPENSE" (reversal credits the expense, reducing the accrued amount)
    pub credit_account: String,

    /// Amount reversed in minor currency units (same as original accrual)
    pub amount_minor: i64,

    /// ISO 4217 currency code (uppercase, e.g. "USD")
    pub currency: String,

    /// Cash flow classification (inherited from original accrual)
    pub cashflow_class: CashFlowClass,

    /// GL journal entry ID created for this reversal
    pub journal_entry_id: Option<Uuid>,

    /// Human-readable reason for the reversal
    /// (e.g. "auto_reverse_next_period", "manual_correction", "period_close")
    pub reason: String,

    pub reversed_at: DateTime<Utc>,
}

/// Build an envelope for gl.accrual_reversed
///
/// mutation_class: REVERSAL (compensates for a prior accrual DATA_MUTATION)
pub fn build_accrual_reversed_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: AccrualReversedPayload,
) -> EventEnvelope<AccrualReversedPayload> {
    create_gl_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_ACCRUAL_REVERSED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_REVERSAL.to_string(),
        payload,
    )
    .with_schema_version(GL_ACCRUAL_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn sample_reversal_policy_auto() -> ReversalPolicy {
        ReversalPolicy {
            auto_reverse_next_period: true,
            reverse_on_date: None,
        }
    }

    fn sample_accrual_created() -> AccrualCreatedPayload {
        AccrualCreatedPayload {
            accrual_id: Uuid::new_v4(),
            template_id: Some(Uuid::new_v4()),
            tenant_id: "tenant-1".to_string(),
            name: "Prepaid Insurance — Jan 2026".to_string(),
            period: "2026-01".to_string(),
            posting_date: "2026-01-01".to_string(),
            debit_account: "PREPAID".to_string(),
            credit_account: "CASH".to_string(),
            amount_minor: 120000, // $1,200.00
            currency: "USD".to_string(),
            cashflow_class: CashFlowClass::Operating,
            reversal_policy: sample_reversal_policy_auto(),
            journal_entry_id: Some(Uuid::new_v4()),
            description: "Monthly insurance prepayment".to_string(),
            created_at: Utc::now(),
        }
    }

    fn sample_accrual_reversed(original_id: Uuid) -> AccrualReversedPayload {
        AccrualReversedPayload {
            reversal_id: Uuid::new_v4(),
            original_accrual_id: original_id,
            template_id: Some(Uuid::new_v4()),
            tenant_id: "tenant-1".to_string(),
            reversal_period: "2026-02".to_string(),
            reversal_date: "2026-02-01".to_string(),
            debit_account: "CASH".to_string(),   // swapped from original credit
            credit_account: "PREPAID".to_string(), // swapped from original debit
            amount_minor: 120000,
            currency: "USD".to_string(),
            cashflow_class: CashFlowClass::Operating,
            journal_entry_id: Some(Uuid::new_v4()),
            reason: "auto_reverse_next_period".to_string(),
            reversed_at: Utc::now(),
        }
    }

    // ─── Event type constants ────────────────────────────────────────────────

    #[test]
    fn event_type_constants_use_gl_prefix() {
        assert!(EVENT_TYPE_ACCRUAL_CREATED.starts_with("gl."));
        assert!(EVENT_TYPE_ACCRUAL_REVERSED.starts_with("gl."));
    }

    #[test]
    fn schema_version_is_stable() {
        assert_eq!(GL_ACCRUAL_SCHEMA_VERSION, "1.0.0");
    }

    // ─── gl.accrual_created envelope ────────────────────────────────────────

    #[test]
    fn accrual_created_envelope_has_correct_type_and_class() {
        let envelope = build_accrual_created_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-1".to_string(),
            None,
            sample_accrual_created(),
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_ACCRUAL_CREATED);
        assert_eq!(
            envelope.mutation_class.as_deref(),
            Some(MUTATION_CLASS_DATA_MUTATION)
        );
        assert_eq!(envelope.schema_version, GL_ACCRUAL_SCHEMA_VERSION);
        assert_eq!(envelope.source_module, "gl");
    }

    #[test]
    fn accrual_created_envelope_carries_causation_id() {
        let envelope = build_accrual_created_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-1".to_string(),
            Some("cause-template-run".to_string()),
            sample_accrual_created(),
        );
        assert_eq!(
            envelope.causation_id.as_deref(),
            Some("cause-template-run")
        );
    }

    #[test]
    fn accrual_created_payload_serializes_correctly() {
        let payload = sample_accrual_created();
        let json = serde_json::to_string(&payload).expect("serialization failed");
        assert!(json.contains("accrual_id"));
        assert!(json.contains("posting_date"));
        assert!(json.contains("debit_account"));
        assert!(json.contains("cashflow_class"));
        assert!(json.contains("reversal_policy"));
    }

    #[test]
    fn accrual_created_cashflow_class_serializes_as_snake_case() {
        let payload = sample_accrual_created(); // Operating
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("\"operating\""));
    }

    // ─── gl.accrual_reversed envelope ───────────────────────────────────────

    #[test]
    fn accrual_reversed_envelope_has_reversal_mutation_class() {
        let original_id = Uuid::new_v4();
        let envelope = build_accrual_reversed_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-2".to_string(),
            Some("cause-accrual-created".to_string()),
            sample_accrual_reversed(original_id),
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_ACCRUAL_REVERSED);
        assert_eq!(
            envelope.mutation_class.as_deref(),
            Some(MUTATION_CLASS_REVERSAL)
        );
        assert_eq!(envelope.schema_version, GL_ACCRUAL_SCHEMA_VERSION);
        assert_eq!(envelope.source_module, "gl");
    }

    #[test]
    fn accrual_reversed_causation_links_to_original_accrual() {
        let original_id = Uuid::new_v4();
        let original_event_id = Uuid::new_v4();
        let envelope = build_accrual_reversed_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-2".to_string(),
            Some(original_event_id.to_string()), // causation = original accrual's event_id
            sample_accrual_reversed(original_id),
        );
        assert_eq!(
            envelope.causation_id.as_deref(),
            Some(original_event_id.to_string().as_str())
        );
        assert_eq!(envelope.payload.original_accrual_id, original_id);
    }

    #[test]
    fn accrual_reversed_payload_swaps_accounts_vs_original() {
        let original = sample_accrual_created();
        let reversed = sample_accrual_reversed(original.accrual_id);

        // Reversal swaps debit/credit to unwind the original entry
        assert_eq!(reversed.debit_account, original.credit_account);
        assert_eq!(reversed.credit_account, original.debit_account);
        assert_eq!(reversed.amount_minor, original.amount_minor);
    }

    // ─── CashFlowClass ───────────────────────────────────────────────────────

    #[test]
    fn cashflow_class_variants_serialize_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&CashFlowClass::Operating).unwrap(),
            "\"operating\""
        );
        assert_eq!(
            serde_json::to_string(&CashFlowClass::Investing).unwrap(),
            "\"investing\""
        );
        assert_eq!(
            serde_json::to_string(&CashFlowClass::Financing).unwrap(),
            "\"financing\""
        );
        assert_eq!(
            serde_json::to_string(&CashFlowClass::NonCash).unwrap(),
            "\"non_cash\""
        );
    }

    #[test]
    fn cashflow_classification_struct_serializes() {
        let classification = CashFlowClassification {
            account_ref: "AR".to_string(),
            cash_flow_class: CashFlowClass::Operating,
            label: "Accounts Receivable (decrease = cash inflow)".to_string(),
            increase_is_outflow: true,
        };
        let json = serde_json::to_string(&classification).unwrap();
        assert!(json.contains("account_ref"));
        assert!(json.contains("\"AR\""));
        assert!(json.contains("\"operating\""));
        assert!(json.contains("increase_is_outflow"));
    }

    // ─── ReversalPolicy ──────────────────────────────────────────────────────

    #[test]
    fn reversal_policy_auto_serializes() {
        let policy = ReversalPolicy {
            auto_reverse_next_period: true,
            reverse_on_date: None,
        };
        let json = serde_json::to_string(&policy).unwrap();
        assert!(json.contains("auto_reverse_next_period"));
        assert!(json.contains("true"));
    }

    #[test]
    fn reversal_policy_explicit_date_serializes() {
        let policy = ReversalPolicy {
            auto_reverse_next_period: false,
            reverse_on_date: Some("2026-03-15".to_string()),
        };
        let json = serde_json::to_string(&policy).unwrap();
        assert!(json.contains("2026-03-15"));
    }

    // ─── Standard cash flow classification mappings ──────────────────────────

    #[test]
    fn standard_account_classifications_are_documented() {
        // These are the canonical account → CashFlowClass mappings.
        // This test serves as living documentation of the classification rules.
        let classifications = vec![
            ("AR", CashFlowClass::Operating),        // Working capital: receivable
            ("AP", CashFlowClass::Operating),         // Working capital: payable
            ("PREPAID", CashFlowClass::Operating),    // Prepaid expense amortization
            ("ACCRUED_LIAB", CashFlowClass::Operating), // Accrued liabilities
            ("PPE", CashFlowClass::Investing),         // Capital expenditure
            ("INVEST", CashFlowClass::Investing),      // Investment purchase
            ("DEBT", CashFlowClass::Financing),        // Debt repayment
            ("EQUITY", CashFlowClass::Financing),      // Equity issuance
            ("DEPR", CashFlowClass::NonCash),          // Depreciation add-back
            ("AMORT", CashFlowClass::NonCash),         // Amortization add-back
        ];

        for (account, class) in classifications {
            let json = serde_json::to_string(&class).unwrap();
            // Verify each class serializes to a known snake_case string
            assert!(
                json.contains("operating")
                    || json.contains("investing")
                    || json.contains("financing")
                    || json.contains("non_cash"),
                "Account '{}' has unknown class serialization: {}",
                account,
                json
            );
        }
    }
}
