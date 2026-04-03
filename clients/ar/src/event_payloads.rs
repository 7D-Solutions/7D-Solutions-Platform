//! Typed event payload structs for AR service events.
//!
//! Consumers can deserialize `EventEnvelope<serde_json::Value>` payloads into
//! these concrete types using `serde_json::from_value`. Each struct matches
//! the canonical schema defined in the AR module's event contracts.

use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ============================================================================
// Event Type Constants
// ============================================================================

// -- Invoice lifecycle --
pub const EVENT_TYPE_INVOICE_OPENED: &str = "ar.invoice_opened";
pub const EVENT_TYPE_INVOICE_PAID: &str = "ar.invoice_paid";

// -- Usage --
pub const EVENT_TYPE_USAGE_CAPTURED: &str = "ar.usage_captured";
pub const EVENT_TYPE_USAGE_INVOICED: &str = "ar.usage_invoiced";

// -- Credit / write-off --
pub const EVENT_TYPE_CREDIT_MEMO_CREATED: &str = "ar.credit_memo_created";
pub const EVENT_TYPE_CREDIT_MEMO_APPROVED: &str = "ar.credit_memo_approved";
pub const EVENT_TYPE_CREDIT_NOTE_ISSUED: &str = "ar.credit_note_issued";
pub const EVENT_TYPE_INVOICE_WRITTEN_OFF: &str = "ar.invoice_written_off";

// -- Aging / dunning --
pub const EVENT_TYPE_AR_AGING_UPDATED: &str = "ar.ar_aging_updated";
pub const EVENT_TYPE_DUNNING_STATE_CHANGED: &str = "ar.dunning_state_changed";
pub const EVENT_TYPE_INVOICE_SUSPENDED: &str = "ar.invoice_suspended";

// -- Tax / FX --
pub const EVENT_TYPE_TAX_QUOTED: &str = "tax.quoted";
pub const EVENT_TYPE_TAX_COMMITTED: &str = "tax.committed";
pub const EVENT_TYPE_TAX_VOIDED: &str = "tax.voided";
pub const EVENT_TYPE_INVOICE_SETTLED_FX: &str = "ar.invoice_settled_fx";

// -- Reconciliation / allocation --
pub const EVENT_TYPE_RECON_RUN_STARTED: &str = "ar.recon_run_started";
pub const EVENT_TYPE_RECON_MATCH_APPLIED: &str = "ar.recon_match_applied";
pub const EVENT_TYPE_RECON_EXCEPTION_RAISED: &str = "ar.recon_exception_raised";
pub const EVENT_TYPE_PAYMENT_ALLOCATED: &str = "ar.payment_allocated";

// -- Progress billing --
pub const EVENT_TYPE_MILESTONE_INVOICE_CREATED: &str = "ar.milestone_invoice_created";

// ============================================================================
// Invoice Lifecycle
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvoiceLifecyclePayload {
    pub invoice_id: String,
    pub customer_id: String,
    pub app_id: String,
    pub amount_cents: i64,
    pub currency: String,
    pub created_at: NaiveDateTime,
    pub due_at: Option<NaiveDateTime>,
    pub paid_at: Option<NaiveDateTime>,
}

// ============================================================================
// Usage
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageCapturedPayload {
    pub usage_id: Uuid,
    pub tenant_id: String,
    pub customer_id: String,
    pub metric_name: String,
    pub quantity: f64,
    pub unit: String,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    pub subscription_id: Option<Uuid>,
    pub captured_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageInvoicedPayload {
    pub usage_id: Uuid,
    pub invoice_id: String,
    pub tenant_id: String,
    pub customer_id: String,
    pub metric_name: String,
    pub quantity: f64,
    pub unit: String,
    pub unit_price_minor: i64,
    pub total_minor: i64,
    pub currency: String,
    pub invoiced_at: DateTime<Utc>,
}

// ============================================================================
// Credit / Write-off
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreditMemoCreatedPayload {
    pub credit_note_id: Uuid,
    pub tenant_id: String,
    pub customer_id: String,
    pub invoice_id: String,
    pub amount_minor: i64,
    pub currency: String,
    pub reason: String,
    pub reference_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreditMemoApprovedPayload {
    pub credit_note_id: Uuid,
    pub tenant_id: String,
    pub invoice_id: String,
    pub approved_by: Option<String>,
    pub approved_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreditNoteIssuedPayload {
    pub credit_note_id: Uuid,
    pub tenant_id: String,
    pub customer_id: String,
    pub invoice_id: String,
    pub amount_minor: i64,
    pub currency: String,
    pub reason: String,
    pub reference_id: Option<String>,
    pub issued_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvoiceWrittenOffPayload {
    pub tenant_id: String,
    pub invoice_id: String,
    pub customer_id: String,
    pub written_off_amount_minor: i64,
    pub currency: String,
    pub reason: String,
    pub authorized_by: Option<String>,
    pub written_off_at: DateTime<Utc>,
}

// ============================================================================
// Aging / Dunning
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgingBuckets {
    pub current_minor: i64,
    pub days_1_30_minor: i64,
    pub days_31_60_minor: i64,
    pub days_61_90_minor: i64,
    pub days_over_90_minor: i64,
    pub total_outstanding_minor: i64,
    pub currency: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArAgingUpdatedPayload {
    pub tenant_id: String,
    pub invoice_count: i64,
    pub buckets: AgingBuckets,
    pub calculated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DunningState {
    Pending,
    Warned,
    Escalated,
    Suspended,
    Resolved,
    WrittenOff,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DunningStateChangedPayload {
    pub tenant_id: String,
    pub invoice_id: String,
    pub customer_id: String,
    pub from_state: Option<DunningState>,
    pub to_state: DunningState,
    pub reason: String,
    pub attempt_number: i32,
    pub next_retry_at: Option<DateTime<Utc>>,
    pub transitioned_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvoiceSuspendedPayload {
    pub tenant_id: String,
    pub invoice_id: String,
    pub customer_id: String,
    pub outstanding_minor: i64,
    pub currency: String,
    pub dunning_attempt: i32,
    pub reason: String,
    pub grace_period_ends_at: Option<DateTime<Utc>>,
    pub suspended_at: DateTime<Utc>,
}

// ============================================================================
// Tax / FX
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaxLineDetail {
    pub line_id: String,
    pub tax_minor: i64,
    pub rate: f64,
    pub jurisdiction: String,
    pub tax_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaxQuotedPayload {
    pub tenant_id: String,
    pub invoice_id: String,
    pub customer_id: String,
    pub total_tax_minor: i64,
    pub currency: String,
    pub tax_by_line: Vec<TaxLineDetail>,
    pub provider_quote_ref: String,
    pub provider: String,
    pub quoted_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaxCommittedPayload {
    pub tenant_id: String,
    pub invoice_id: String,
    pub customer_id: String,
    pub total_tax_minor: i64,
    pub currency: String,
    pub provider_quote_ref: String,
    pub provider_commit_ref: String,
    pub provider: String,
    pub committed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaxVoidedPayload {
    pub tenant_id: String,
    pub invoice_id: String,
    pub customer_id: String,
    pub total_tax_minor: i64,
    pub currency: String,
    pub provider_commit_ref: String,
    pub provider: String,
    pub void_reason: String,
    pub voided_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvoiceSettledFxPayload {
    pub tenant_id: String,
    pub invoice_id: String,
    pub customer_id: String,
    pub txn_currency: String,
    pub txn_amount_minor: i64,
    pub rpt_currency: String,
    pub recognition_rpt_amount_minor: i64,
    pub recognition_rate_id: Uuid,
    pub recognition_rate: f64,
    pub settlement_rpt_amount_minor: i64,
    pub settlement_rate_id: Uuid,
    pub settlement_rate: f64,
    pub realized_gain_loss_minor: i64,
    pub settled_at: DateTime<Utc>,
}

// ============================================================================
// Reconciliation / Allocation
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReconRunStartedPayload {
    pub tenant_id: String,
    pub recon_run_id: Uuid,
    pub payment_count: i32,
    pub invoice_count: i32,
    pub matching_strategy: String,
    pub started_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReconMatchAppliedPayload {
    pub tenant_id: String,
    pub recon_run_id: Uuid,
    pub payment_id: String,
    pub invoice_id: String,
    pub matched_amount_minor: i64,
    pub currency: String,
    pub confidence_score: f64,
    pub match_method: String,
    pub matched_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReconExceptionKind {
    UnmatchedPayment,
    UnmatchedInvoice,
    AmountMismatch,
    AmbiguousMatch,
    DuplicateReference,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReconExceptionRaisedPayload {
    pub tenant_id: String,
    pub recon_run_id: Uuid,
    pub payment_id: Option<String>,
    pub invoice_id: Option<String>,
    pub exception_kind: ReconExceptionKind,
    pub description: String,
    pub amount_minor: Option<i64>,
    pub currency: Option<String>,
    pub raised_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllocationLine {
    pub invoice_id: String,
    pub allocated_minor: i64,
    pub remaining_after_minor: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentAllocatedPayload {
    pub tenant_id: String,
    pub payment_id: String,
    pub customer_id: String,
    pub payment_amount_minor: i64,
    pub allocated_amount_minor: i64,
    pub unallocated_amount_minor: i64,
    pub currency: String,
    pub allocation_strategy: String,
    pub allocations: Vec<AllocationLine>,
    pub allocated_at: DateTime<Utc>,
}

// ============================================================================
// Progress Billing
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MilestoneInvoiceCreatedPayload {
    pub contract_id: Uuid,
    pub milestone_id: Uuid,
    pub tenant_id: String,
    pub customer_id: String,
    pub invoice_id: i32,
    pub amount_minor: i64,
    pub currency: String,
    pub milestone_name: String,
    pub percentage: i32,
    pub cumulative_billed_minor: i64,
    pub contract_total_minor: i64,
    pub created_at: DateTime<Utc>,
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_type_constants_match_canonical_values() {
        assert_eq!(EVENT_TYPE_INVOICE_OPENED, "ar.invoice_opened");
        assert_eq!(EVENT_TYPE_INVOICE_PAID, "ar.invoice_paid");
        assert_eq!(EVENT_TYPE_USAGE_CAPTURED, "ar.usage_captured");
        assert_eq!(EVENT_TYPE_USAGE_INVOICED, "ar.usage_invoiced");
        assert_eq!(EVENT_TYPE_CREDIT_MEMO_CREATED, "ar.credit_memo_created");
        assert_eq!(EVENT_TYPE_CREDIT_NOTE_ISSUED, "ar.credit_note_issued");
        assert_eq!(EVENT_TYPE_INVOICE_WRITTEN_OFF, "ar.invoice_written_off");
        assert_eq!(EVENT_TYPE_AR_AGING_UPDATED, "ar.ar_aging_updated");
        assert_eq!(EVENT_TYPE_DUNNING_STATE_CHANGED, "ar.dunning_state_changed");
        assert_eq!(EVENT_TYPE_INVOICE_SUSPENDED, "ar.invoice_suspended");
        assert_eq!(EVENT_TYPE_TAX_QUOTED, "tax.quoted");
        assert_eq!(EVENT_TYPE_TAX_COMMITTED, "tax.committed");
        assert_eq!(EVENT_TYPE_TAX_VOIDED, "tax.voided");
        assert_eq!(EVENT_TYPE_INVOICE_SETTLED_FX, "ar.invoice_settled_fx");
        assert_eq!(EVENT_TYPE_RECON_RUN_STARTED, "ar.recon_run_started");
        assert_eq!(EVENT_TYPE_PAYMENT_ALLOCATED, "ar.payment_allocated");
        assert_eq!(EVENT_TYPE_MILESTONE_INVOICE_CREATED, "ar.milestone_invoice_created");
    }

    #[test]
    fn invoice_lifecycle_payload_roundtrips() {
        let payload = InvoiceLifecyclePayload {
            invoice_id: "42".to_string(),
            customer_id: "7".to_string(),
            app_id: "test-app".to_string(),
            amount_cents: 5000,
            currency: "usd".to_string(),
            created_at: chrono::Utc::now().naive_utc(),
            due_at: None,
            paid_at: None,
        };
        let json = serde_json::to_string(&payload).unwrap();
        let _: InvoiceLifecyclePayload = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn dunning_state_serializes_as_snake_case() {
        assert_eq!(serde_json::to_string(&DunningState::WrittenOff).unwrap(), "\"written_off\"");
        assert_eq!(serde_json::to_string(&DunningState::Pending).unwrap(), "\"pending\"");
    }

    #[test]
    fn recon_exception_kind_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&ReconExceptionKind::UnmatchedPayment).unwrap(),
            "\"unmatched_payment\""
        );
        assert_eq!(
            serde_json::to_string(&ReconExceptionKind::AmbiguousMatch).unwrap(),
            "\"ambiguous_match\""
        );
    }

    #[test]
    fn tax_quoted_payload_roundtrips() {
        let payload = TaxQuotedPayload {
            tenant_id: "t".to_string(),
            invoice_id: "i".to_string(),
            customer_id: "c".to_string(),
            total_tax_minor: 850,
            currency: "usd".to_string(),
            tax_by_line: vec![TaxLineDetail {
                line_id: "l1".to_string(),
                tax_minor: 850,
                rate: 0.085,
                jurisdiction: "CA".to_string(),
                tax_type: "sales_tax".to_string(),
            }],
            provider_quote_ref: "q".to_string(),
            provider: "local".to_string(),
            quoted_at: chrono::Utc::now(),
        };
        let json = serde_json::to_string(&payload).unwrap();
        let _: TaxQuotedPayload = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn payment_allocated_payload_roundtrips() {
        let payload = PaymentAllocatedPayload {
            tenant_id: "t".to_string(),
            payment_id: "p".to_string(),
            customer_id: "c".to_string(),
            payment_amount_minor: 1000,
            allocated_amount_minor: 1000,
            unallocated_amount_minor: 0,
            currency: "usd".to_string(),
            allocation_strategy: "fifo".to_string(),
            allocations: vec![AllocationLine {
                invoice_id: "inv-1".to_string(),
                allocated_minor: 1000,
                remaining_after_minor: 0,
            }],
            allocated_at: chrono::Utc::now(),
        };
        let json = serde_json::to_string(&payload).unwrap();
        let _: PaymentAllocatedPayload = serde_json::from_str(&json).unwrap();
    }
}
