//! AR event type constants and payload structs
//!
//! Defines the canonical event contracts for AR's monetization events:
//! - ar.usage_captured       (metered usage recorded)
//! - ar.usage_invoiced       (usage billed on an invoice)
//! - ar.credit_note_issued   (credit note issued against an invoice)
//! - ar.invoice_written_off  (invoice written off as bad debt)
//! - ar.ar_aging_updated     (AR aging projection updated)
//!
//! All events carry a full EventEnvelope with:
//! - schema_version: "1.0.0" (stable for this event version)
//! - mutation_class: per event (DATA_MUTATION or REVERSAL)
//! - correlation_id / causation_id: caller-supplied for tracing
//! - event_id: caller-supplied for idempotency (deterministic from business key)

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::events::envelope::{create_ar_envelope, EventEnvelope};

// ============================================================================
// Event Type Constants
// ============================================================================

/// Metered usage was captured for billing (idempotent per usage_id)
pub const EVENT_TYPE_USAGE_CAPTURED: &str = "ar.usage_captured";

/// Captured usage was billed on an invoice line item
pub const EVENT_TYPE_USAGE_INVOICED: &str = "ar.usage_invoiced";

/// A credit note was formally issued against an invoice
pub const EVENT_TYPE_CREDIT_NOTE_ISSUED: &str = "ar.credit_note_issued";

/// An invoice was written off as uncollectable bad debt
pub const EVENT_TYPE_INVOICE_WRITTEN_OFF: &str = "ar.invoice_written_off";

/// AR aging buckets were updated (projection refresh)
pub const EVENT_TYPE_AR_AGING_UPDATED: &str = "ar.ar_aging_updated";

// ============================================================================
// Schema Version
// ============================================================================

/// Schema version for all AR monetization event payloads (v1)
pub const AR_EVENT_SCHEMA_VERSION: &str = "1.0.0";

// ============================================================================
// Mutation Classes
// ============================================================================

/// DATA_MUTATION: creates or modifies a financial record
pub const MUTATION_CLASS_DATA_MUTATION: &str = "DATA_MUTATION";

/// REVERSAL: compensates for a prior DATA_MUTATION (write-off, void)
pub const MUTATION_CLASS_REVERSAL: &str = "REVERSAL";

/// LIFECYCLE: entity lifecycle transitions (dunning state changes, suspension)
pub const MUTATION_CLASS_LIFECYCLE: &str = "LIFECYCLE";

// ============================================================================
// Payload: ar.usage_captured
// ============================================================================

/// Payload for ar.usage_captured
///
/// Emitted when metered usage is recorded for a tenant/customer.
/// Idempotency: caller MUST supply a deterministic event_id derived from usage_id.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageCapturedPayload {
    /// Stable business key for this usage record (idempotency anchor)
    pub usage_id: Uuid,
    pub tenant_id: String,
    pub customer_id: String,
    /// Metric being measured (e.g. "api_calls", "gb_storage")
    pub metric_name: String,
    /// Quantity consumed
    pub quantity: f64,
    /// Unit for the quantity (e.g. "calls", "GB")
    pub unit: String,
    /// Start of the usage measurement period
    pub period_start: DateTime<Utc>,
    /// End of the usage measurement period
    pub period_end: DateTime<Utc>,
    /// Subscription this usage belongs to (if applicable)
    pub subscription_id: Option<Uuid>,
    pub captured_at: DateTime<Utc>,
}

/// Build an envelope for ar.usage_captured
///
/// mutation_class: DATA_MUTATION (new usage record)
pub fn build_usage_captured_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: UsageCapturedPayload,
) -> EventEnvelope<UsageCapturedPayload> {
    create_ar_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_USAGE_CAPTURED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(AR_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Payload: ar.usage_invoiced
// ============================================================================

/// Payload for ar.usage_invoiced
///
/// Emitted when captured usage is billed as a line item on an invoice.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageInvoicedPayload {
    pub usage_id: Uuid,
    pub invoice_id: String,
    pub tenant_id: String,
    pub customer_id: String,
    pub metric_name: String,
    pub quantity: f64,
    pub unit: String,
    /// Unit price in minor currency units (e.g. cents)
    pub unit_price_minor: i64,
    /// Total billed amount in minor currency units
    pub total_minor: i64,
    pub currency: String,
    pub invoiced_at: DateTime<Utc>,
}

/// Build an envelope for ar.usage_invoiced
///
/// mutation_class: DATA_MUTATION (new invoice line item)
pub fn build_usage_invoiced_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: UsageInvoicedPayload,
) -> EventEnvelope<UsageInvoicedPayload> {
    create_ar_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_USAGE_INVOICED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(AR_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Payload: ar.credit_note_issued
// ============================================================================

/// Payload for ar.credit_note_issued
///
/// Emitted when a formal credit note is issued against an invoice.
/// Credit notes reduce the amount owed without voiding the original invoice.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreditNoteIssuedPayload {
    /// Stable business key for this credit note
    pub credit_note_id: Uuid,
    pub tenant_id: String,
    pub customer_id: String,
    /// Invoice this credit note is applied against
    pub invoice_id: String,
    /// Credit amount in minor currency units (positive = reduces balance)
    pub amount_minor: i64,
    pub currency: String,
    pub reason: String,
    /// Optional reference to original usage or line item
    pub reference_id: Option<String>,
    pub issued_at: DateTime<Utc>,
}

/// Build an envelope for ar.credit_note_issued
///
/// mutation_class: DATA_MUTATION (creates a new credit note artifact)
pub fn build_credit_note_issued_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: CreditNoteIssuedPayload,
) -> EventEnvelope<CreditNoteIssuedPayload> {
    create_ar_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_CREDIT_NOTE_ISSUED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(AR_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Payload: ar.invoice_written_off
// ============================================================================

/// Payload for ar.invoice_written_off
///
/// Emitted when an invoice is written off as uncollectable bad debt.
/// Write-off is a financial reversal of the receivable — the debt is forgiven.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvoiceWrittenOffPayload {
    pub tenant_id: String,
    pub invoice_id: String,
    pub customer_id: String,
    /// Outstanding amount written off in minor currency units
    pub written_off_amount_minor: i64,
    pub currency: String,
    /// Reason for write-off (e.g. "uncollectable", "bankruptcy", "dispute_settled")
    pub reason: String,
    /// Actor who authorized the write-off
    pub authorized_by: Option<String>,
    pub written_off_at: DateTime<Utc>,
}

/// Build an envelope for ar.invoice_written_off
///
/// mutation_class: REVERSAL (compensates for the original receivable)
pub fn build_invoice_written_off_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: InvoiceWrittenOffPayload,
) -> EventEnvelope<InvoiceWrittenOffPayload> {
    create_ar_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_INVOICE_WRITTEN_OFF.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_REVERSAL.to_string(),
        payload,
    )
    .with_schema_version(AR_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Payload: ar.ar_aging_updated
// ============================================================================

/// Summary of outstanding balances by aging bucket
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgingBuckets {
    /// Invoices not yet due (amount_minor)
    pub current_minor: i64,
    /// 1–30 days overdue
    pub days_1_30_minor: i64,
    /// 31–60 days overdue
    pub days_31_60_minor: i64,
    /// 61–90 days overdue
    pub days_61_90_minor: i64,
    /// Over 90 days overdue
    pub days_over_90_minor: i64,
    /// Total outstanding (sum of all buckets)
    pub total_outstanding_minor: i64,
    pub currency: String,
}

/// Payload for ar.ar_aging_updated
///
/// Emitted when the AR aging projection is refreshed for a tenant.
/// Captures point-in-time aging buckets for the tenant's receivables.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArAgingUpdatedPayload {
    pub tenant_id: String,
    /// Invoice count included in this snapshot
    pub invoice_count: i64,
    pub buckets: AgingBuckets,
    /// Timestamp of the aging calculation (as-of date)
    pub calculated_at: DateTime<Utc>,
}

/// Build an envelope for ar.ar_aging_updated
///
/// mutation_class: DATA_MUTATION (creates/updates an aging projection record)
pub fn build_ar_aging_updated_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ArAgingUpdatedPayload,
) -> EventEnvelope<ArAgingUpdatedPayload> {
    create_ar_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_AR_AGING_UPDATED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(AR_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Phase 22 Event Type Constants
// ============================================================================

/// Dunning state machine transitioned (e.g. pending → warned → suspended → resolved)
pub const EVENT_TYPE_DUNNING_STATE_CHANGED: &str = "ar.dunning_state_changed";

/// Invoice suspended due to non-payment escalation
pub const EVENT_TYPE_INVOICE_SUSPENDED: &str = "ar.invoice_suspended";

/// A reconciliation run was initiated
pub const EVENT_TYPE_RECON_RUN_STARTED: &str = "ar.recon_run_started";

/// A reconciliation match was successfully applied (payment ↔ invoice)
pub const EVENT_TYPE_RECON_MATCH_APPLIED: &str = "ar.recon_match_applied";

/// A reconciliation exception was raised (unmatched or ambiguous)
pub const EVENT_TYPE_RECON_EXCEPTION_RAISED: &str = "ar.recon_exception_raised";

/// A payment was allocated to one or more invoices
pub const EVENT_TYPE_PAYMENT_ALLOCATED: &str = "ar.payment_allocated";

// ============================================================================
// Payload: ar.dunning_state_changed
// ============================================================================

/// Dunning state machine values
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

/// Payload for ar.dunning_state_changed
///
/// Emitted when the dunning state machine transitions for an invoice or customer.
/// Idempotency: caller MUST supply a deterministic event_id derived from (invoice_id, to_state, occurred_at).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DunningStateChangedPayload {
    pub tenant_id: String,
    pub invoice_id: String,
    pub customer_id: String,
    /// Previous dunning state (None if this is the initial state)
    pub from_state: Option<DunningState>,
    /// New dunning state after transition
    pub to_state: DunningState,
    /// Human-readable reason for the transition
    pub reason: String,
    /// Which dunning attempt number triggered this (1-indexed)
    pub attempt_number: i32,
    /// Next retry scheduled at (None if terminal state)
    pub next_retry_at: Option<DateTime<Utc>>,
    pub transitioned_at: DateTime<Utc>,
}

/// Build an envelope for ar.dunning_state_changed
///
/// mutation_class: LIFECYCLE (state machine transition)
pub fn build_dunning_state_changed_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: DunningStateChangedPayload,
) -> EventEnvelope<DunningStateChangedPayload> {
    create_ar_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_DUNNING_STATE_CHANGED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_LIFECYCLE.to_string(),
        payload,
    )
    .with_schema_version(AR_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Payload: ar.invoice_suspended
// ============================================================================

/// Payload for ar.invoice_suspended
///
/// Emitted when an invoice is formally suspended due to dunning escalation.
/// Suspension may trigger service interruption upstream.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvoiceSuspendedPayload {
    pub tenant_id: String,
    pub invoice_id: String,
    pub customer_id: String,
    /// Outstanding balance at the time of suspension (minor currency units)
    pub outstanding_minor: i64,
    pub currency: String,
    /// Which dunning attempt number triggered suspension
    pub dunning_attempt: i32,
    /// Reason for suspension
    pub reason: String,
    /// Suspension may be lifted if payment received before this date
    pub grace_period_ends_at: Option<DateTime<Utc>>,
    pub suspended_at: DateTime<Utc>,
}

/// Build an envelope for ar.invoice_suspended
///
/// mutation_class: LIFECYCLE (invoice enters suspended lifecycle state)
pub fn build_invoice_suspended_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: InvoiceSuspendedPayload,
) -> EventEnvelope<InvoiceSuspendedPayload> {
    create_ar_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_INVOICE_SUSPENDED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_LIFECYCLE.to_string(),
        payload,
    )
    .with_schema_version(AR_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Payload: ar.recon_run_started
// ============================================================================

/// Payload for ar.recon_run_started
///
/// Emitted when a reconciliation run is initiated. Anchors causation chain
/// for all match/exception events that follow in the same run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReconRunStartedPayload {
    pub tenant_id: String,
    /// Stable ID for this reconciliation run (idempotency anchor)
    pub recon_run_id: Uuid,
    /// Number of payments included in this run
    pub payment_count: i32,
    /// Number of invoices considered for matching
    pub invoice_count: i32,
    /// Strategy used for matching (e.g. "fifo", "exact_match", "best_fit")
    pub matching_strategy: String,
    pub started_at: DateTime<Utc>,
}

/// Build an envelope for ar.recon_run_started
///
/// mutation_class: DATA_MUTATION (creates a reconciliation run record)
pub fn build_recon_run_started_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ReconRunStartedPayload,
) -> EventEnvelope<ReconRunStartedPayload> {
    create_ar_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_RECON_RUN_STARTED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(AR_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Payload: ar.recon_match_applied
// ============================================================================

/// Payload for ar.recon_match_applied
///
/// Emitted when a reconciliation match is confirmed and applied.
/// A single payment may generate multiple match events (partial allocations).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReconMatchAppliedPayload {
    pub tenant_id: String,
    pub recon_run_id: Uuid,
    pub payment_id: String,
    pub invoice_id: String,
    /// Amount applied in this match (minor currency units)
    pub matched_amount_minor: i64,
    pub currency: String,
    /// Confidence score 0.0–1.0 (1.0 = exact match)
    pub confidence_score: f64,
    /// Match method applied (e.g. "exact", "fifo", "reference")
    pub match_method: String,
    pub matched_at: DateTime<Utc>,
}

/// Build an envelope for ar.recon_match_applied
///
/// mutation_class: DATA_MUTATION (creates a reconciliation match record)
pub fn build_recon_match_applied_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ReconMatchAppliedPayload,
) -> EventEnvelope<ReconMatchAppliedPayload> {
    create_ar_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_RECON_MATCH_APPLIED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(AR_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Payload: ar.recon_exception_raised
// ============================================================================

/// Classification of reconciliation exceptions
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReconExceptionKind {
    /// Payment received but no matching invoice found
    UnmatchedPayment,
    /// Invoice overdue but no corresponding payment
    UnmatchedInvoice,
    /// Payment amount does not match any invoice amount
    AmountMismatch,
    /// Multiple invoices match with equal confidence
    AmbiguousMatch,
    /// Duplicate payment reference detected
    DuplicateReference,
}

/// Payload for ar.recon_exception_raised
///
/// Emitted when a reconciliation exception cannot be automatically resolved.
/// Requires manual review or escalation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReconExceptionRaisedPayload {
    pub tenant_id: String,
    pub recon_run_id: Uuid,
    /// Payment ID if the exception is payment-related (optional for invoice-only exceptions)
    pub payment_id: Option<String>,
    /// Invoice ID if the exception is invoice-related
    pub invoice_id: Option<String>,
    pub exception_kind: ReconExceptionKind,
    /// Human-readable description of the exception
    pub description: String,
    /// Amount involved in minor currency units (if applicable)
    pub amount_minor: Option<i64>,
    pub currency: Option<String>,
    pub raised_at: DateTime<Utc>,
}

/// Build an envelope for ar.recon_exception_raised
///
/// mutation_class: DATA_MUTATION (creates an exception record for manual review)
pub fn build_recon_exception_raised_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ReconExceptionRaisedPayload,
) -> EventEnvelope<ReconExceptionRaisedPayload> {
    create_ar_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_RECON_EXCEPTION_RAISED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(AR_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Payload: ar.payment_allocated
// ============================================================================

/// A single allocation line: one invoice portion of the payment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllocationLine {
    pub invoice_id: String,
    /// Amount allocated to this invoice (minor currency units)
    pub allocated_minor: i64,
    /// Remaining balance on this invoice after allocation (minor currency units)
    pub remaining_after_minor: i64,
}

/// Payload for ar.payment_allocated
///
/// Emitted when a payment is allocated (partially or fully) to one or more invoices.
/// Uses FIFO by default; allocation strategy is included for auditability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentAllocatedPayload {
    pub tenant_id: String,
    pub payment_id: String,
    pub customer_id: String,
    /// Total payment amount (minor currency units)
    pub payment_amount_minor: i64,
    /// Amount allocated in this event (may be partial)
    pub allocated_amount_minor: i64,
    /// Amount remaining unallocated after this event
    pub unallocated_amount_minor: i64,
    pub currency: String,
    /// Allocation strategy applied (e.g. "fifo", "specific", "proportional")
    pub allocation_strategy: String,
    /// Invoice allocations in this event (ordered by application)
    pub allocations: Vec<AllocationLine>,
    pub allocated_at: DateTime<Utc>,
}

/// Build an envelope for ar.payment_allocated
///
/// mutation_class: DATA_MUTATION (creates allocation records against invoices)
pub fn build_payment_allocated_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: PaymentAllocatedPayload,
) -> EventEnvelope<PaymentAllocatedPayload> {
    create_ar_envelope(
        event_id,
        tenant_id,
        EVENT_TYPE_PAYMENT_ALLOCATED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(AR_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Phase 23b Tax Event Type Constants
// ============================================================================

/// Tax was calculated for an invoice draft (pre-commit, reversible)
pub const EVENT_TYPE_TAX_QUOTED: &str = "tax.quoted";

/// Tax was committed when invoice was finalized (legally due)
pub const EVENT_TYPE_TAX_COMMITTED: &str = "tax.committed";

/// A committed tax transaction was voided (refund, write-off, or cancellation)
pub const EVENT_TYPE_TAX_VOIDED: &str = "tax.voided";

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
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn sample_usage_captured() -> UsageCapturedPayload {
        UsageCapturedPayload {
            usage_id: Uuid::new_v4(),
            tenant_id: "tenant-1".to_string(),
            customer_id: "cust-1".to_string(),
            metric_name: "api_calls".to_string(),
            quantity: 1500.0,
            unit: "calls".to_string(),
            period_start: Utc::now(),
            period_end: Utc::now(),
            subscription_id: None,
            captured_at: Utc::now(),
        }
    }

    #[test]
    fn usage_captured_envelope_has_correct_type_and_class() {
        let envelope = build_usage_captured_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-1".to_string(),
            None,
            sample_usage_captured(),
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_USAGE_CAPTURED);
        assert_eq!(
            envelope.mutation_class.as_deref(),
            Some(MUTATION_CLASS_DATA_MUTATION)
        );
        assert_eq!(envelope.schema_version, AR_EVENT_SCHEMA_VERSION);
        assert_eq!(envelope.source_module, "ar");
    }

    #[test]
    fn usage_invoiced_envelope_has_correct_metadata() {
        let payload = UsageInvoicedPayload {
            usage_id: Uuid::new_v4(),
            invoice_id: "inv-42".to_string(),
            tenant_id: "tenant-1".to_string(),
            customer_id: "cust-1".to_string(),
            metric_name: "api_calls".to_string(),
            quantity: 1500.0,
            unit: "calls".to_string(),
            unit_price_minor: 10,
            total_minor: 15000,
            currency: "usd".to_string(),
            invoiced_at: Utc::now(),
        };
        let envelope = build_usage_invoiced_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-1".to_string(),
            Some("cause-1".to_string()),
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_USAGE_INVOICED);
        assert_eq!(
            envelope.mutation_class.as_deref(),
            Some(MUTATION_CLASS_DATA_MUTATION)
        );
        assert_eq!(envelope.causation_id.as_deref(), Some("cause-1"));
    }

    #[test]
    fn credit_note_envelope_has_correct_metadata() {
        let payload = CreditNoteIssuedPayload {
            credit_note_id: Uuid::new_v4(),
            tenant_id: "tenant-1".to_string(),
            customer_id: "cust-1".to_string(),
            invoice_id: "inv-99".to_string(),
            amount_minor: 5000,
            currency: "usd".to_string(),
            reason: "service_credit".to_string(),
            reference_id: None,
            issued_at: Utc::now(),
        };
        let envelope = build_credit_note_issued_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-1".to_string(),
            None,
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_CREDIT_NOTE_ISSUED);
        assert_eq!(
            envelope.mutation_class.as_deref(),
            Some(MUTATION_CLASS_DATA_MUTATION)
        );
        assert_eq!(envelope.schema_version, AR_EVENT_SCHEMA_VERSION);
    }

    #[test]
    fn write_off_envelope_has_reversal_mutation_class() {
        let payload = InvoiceWrittenOffPayload {
            tenant_id: "tenant-1".to_string(),
            invoice_id: "inv-77".to_string(),
            customer_id: "cust-1".to_string(),
            written_off_amount_minor: 20000,
            currency: "usd".to_string(),
            reason: "uncollectable".to_string(),
            authorized_by: Some("admin@tenant.local".to_string()),
            written_off_at: Utc::now(),
        };
        let envelope = build_invoice_written_off_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-1".to_string(),
            None,
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_INVOICE_WRITTEN_OFF);
        // Write-off is a REVERSAL of the receivable
        assert_eq!(
            envelope.mutation_class.as_deref(),
            Some(MUTATION_CLASS_REVERSAL)
        );
        assert_eq!(envelope.schema_version, AR_EVENT_SCHEMA_VERSION);
    }

    #[test]
    fn aging_updated_envelope_has_correct_metadata() {
        let payload = ArAgingUpdatedPayload {
            tenant_id: "tenant-1".to_string(),
            invoice_count: 12,
            buckets: AgingBuckets {
                current_minor: 100000,
                days_1_30_minor: 50000,
                days_31_60_minor: 20000,
                days_61_90_minor: 5000,
                days_over_90_minor: 2000,
                total_outstanding_minor: 177000,
                currency: "usd".to_string(),
            },
            calculated_at: Utc::now(),
        };
        let envelope = build_ar_aging_updated_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-1".to_string(),
            None,
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_AR_AGING_UPDATED);
        assert_eq!(
            envelope.mutation_class.as_deref(),
            Some(MUTATION_CLASS_DATA_MUTATION)
        );
        assert_eq!(envelope.schema_version, AR_EVENT_SCHEMA_VERSION);
    }

    #[test]
    fn all_envelopes_have_stable_schema_version() {
        assert_eq!(AR_EVENT_SCHEMA_VERSION, "1.0.0");
    }

    #[test]
    fn all_event_type_constants_use_ar_prefix() {
        assert!(EVENT_TYPE_USAGE_CAPTURED.starts_with("ar."));
        assert!(EVENT_TYPE_USAGE_INVOICED.starts_with("ar."));
        assert!(EVENT_TYPE_CREDIT_NOTE_ISSUED.starts_with("ar."));
        assert!(EVENT_TYPE_INVOICE_WRITTEN_OFF.starts_with("ar."));
        assert!(EVENT_TYPE_AR_AGING_UPDATED.starts_with("ar."));
    }

    #[test]
    fn usage_captured_payload_serializes_correctly() {
        let payload = sample_usage_captured();
        let json = serde_json::to_string(&payload).expect("serialization failed");
        assert!(json.contains("usage_id"));
        assert!(json.contains("metric_name"));
        assert!(json.contains("api_calls"));
    }

    #[test]
    fn aging_buckets_total_fields_present() {
        let buckets = AgingBuckets {
            current_minor: 1000,
            days_1_30_minor: 500,
            days_31_60_minor: 200,
            days_61_90_minor: 100,
            days_over_90_minor: 50,
            total_outstanding_minor: 1850,
            currency: "usd".to_string(),
        };
        let json = serde_json::to_string(&buckets).expect("serialization failed");
        assert!(json.contains("total_outstanding_minor"));
        assert!(json.contains("days_over_90_minor"));
    }

    // ============================================================================
    // Phase 22 tests
    // ============================================================================

    #[test]
    fn dunning_state_changed_envelope_has_lifecycle_class() {
        let payload = DunningStateChangedPayload {
            tenant_id: "tenant-1".to_string(),
            invoice_id: "inv-100".to_string(),
            customer_id: "cust-1".to_string(),
            from_state: Some(DunningState::Pending),
            to_state: DunningState::Warned,
            reason: "first_overdue_notice".to_string(),
            attempt_number: 1,
            next_retry_at: Some(Utc::now()),
            transitioned_at: Utc::now(),
        };
        let envelope = build_dunning_state_changed_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-1".to_string(),
            None,
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_DUNNING_STATE_CHANGED);
        assert_eq!(
            envelope.mutation_class.as_deref(),
            Some(MUTATION_CLASS_LIFECYCLE)
        );
        assert_eq!(envelope.schema_version, AR_EVENT_SCHEMA_VERSION);
        assert_eq!(envelope.source_module, "ar");
    }

    #[test]
    fn invoice_suspended_envelope_has_lifecycle_class() {
        let payload = InvoiceSuspendedPayload {
            tenant_id: "tenant-1".to_string(),
            invoice_id: "inv-101".to_string(),
            customer_id: "cust-1".to_string(),
            outstanding_minor: 50000,
            currency: "usd".to_string(),
            dunning_attempt: 3,
            reason: "max_attempts_exceeded".to_string(),
            grace_period_ends_at: None,
            suspended_at: Utc::now(),
        };
        let envelope = build_invoice_suspended_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-1".to_string(),
            Some("cause-dunning".to_string()),
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_INVOICE_SUSPENDED);
        assert_eq!(
            envelope.mutation_class.as_deref(),
            Some(MUTATION_CLASS_LIFECYCLE)
        );
        assert_eq!(envelope.causation_id.as_deref(), Some("cause-dunning"));
    }

    #[test]
    fn recon_run_started_envelope_has_data_mutation_class() {
        let run_id = Uuid::new_v4();
        let payload = ReconRunStartedPayload {
            tenant_id: "tenant-1".to_string(),
            recon_run_id: run_id,
            payment_count: 10,
            invoice_count: 25,
            matching_strategy: "fifo".to_string(),
            started_at: Utc::now(),
        };
        let envelope = build_recon_run_started_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-1".to_string(),
            None,
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_RECON_RUN_STARTED);
        assert_eq!(
            envelope.mutation_class.as_deref(),
            Some(MUTATION_CLASS_DATA_MUTATION)
        );
        assert_eq!(envelope.schema_version, AR_EVENT_SCHEMA_VERSION);
    }

    #[test]
    fn recon_match_applied_envelope_has_correct_metadata() {
        let payload = ReconMatchAppliedPayload {
            tenant_id: "tenant-1".to_string(),
            recon_run_id: Uuid::new_v4(),
            payment_id: "pay-55".to_string(),
            invoice_id: "inv-55".to_string(),
            matched_amount_minor: 10000,
            currency: "usd".to_string(),
            confidence_score: 1.0,
            match_method: "exact".to_string(),
            matched_at: Utc::now(),
        };
        let envelope = build_recon_match_applied_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-1".to_string(),
            None,
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_RECON_MATCH_APPLIED);
        assert_eq!(
            envelope.mutation_class.as_deref(),
            Some(MUTATION_CLASS_DATA_MUTATION)
        );
    }

    #[test]
    fn recon_exception_raised_envelope_has_correct_metadata() {
        let payload = ReconExceptionRaisedPayload {
            tenant_id: "tenant-1".to_string(),
            recon_run_id: Uuid::new_v4(),
            payment_id: Some("pay-99".to_string()),
            invoice_id: None,
            exception_kind: ReconExceptionKind::UnmatchedPayment,
            description: "No invoice found for payment reference PAY-99".to_string(),
            amount_minor: Some(20000),
            currency: Some("usd".to_string()),
            raised_at: Utc::now(),
        };
        let envelope = build_recon_exception_raised_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-1".to_string(),
            None,
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_RECON_EXCEPTION_RAISED);
        assert_eq!(
            envelope.mutation_class.as_deref(),
            Some(MUTATION_CLASS_DATA_MUTATION)
        );
        assert_eq!(envelope.schema_version, AR_EVENT_SCHEMA_VERSION);
    }

    #[test]
    fn payment_allocated_envelope_has_correct_metadata() {
        let payload = PaymentAllocatedPayload {
            tenant_id: "tenant-1".to_string(),
            payment_id: "pay-42".to_string(),
            customer_id: "cust-1".to_string(),
            payment_amount_minor: 30000,
            allocated_amount_minor: 25000,
            unallocated_amount_minor: 5000,
            currency: "usd".to_string(),
            allocation_strategy: "fifo".to_string(),
            allocations: vec![
                AllocationLine {
                    invoice_id: "inv-1".to_string(),
                    allocated_minor: 15000,
                    remaining_after_minor: 0,
                },
                AllocationLine {
                    invoice_id: "inv-2".to_string(),
                    allocated_minor: 10000,
                    remaining_after_minor: 5000,
                },
            ],
            allocated_at: Utc::now(),
        };
        let envelope = build_payment_allocated_envelope(
            Uuid::new_v4(),
            "tenant-1".to_string(),
            "corr-1".to_string(),
            None,
            payload,
        );
        assert_eq!(envelope.event_type, EVENT_TYPE_PAYMENT_ALLOCATED);
        assert_eq!(
            envelope.mutation_class.as_deref(),
            Some(MUTATION_CLASS_DATA_MUTATION)
        );
        assert_eq!(envelope.schema_version, AR_EVENT_SCHEMA_VERSION);
    }

    #[test]
    fn phase22_event_type_constants_use_ar_prefix() {
        assert!(EVENT_TYPE_DUNNING_STATE_CHANGED.starts_with("ar."));
        assert!(EVENT_TYPE_INVOICE_SUSPENDED.starts_with("ar."));
        assert!(EVENT_TYPE_RECON_RUN_STARTED.starts_with("ar."));
        assert!(EVENT_TYPE_RECON_MATCH_APPLIED.starts_with("ar."));
        assert!(EVENT_TYPE_RECON_EXCEPTION_RAISED.starts_with("ar."));
        assert!(EVENT_TYPE_PAYMENT_ALLOCATED.starts_with("ar."));
    }

    #[test]
    fn dunning_state_serializes_as_snake_case() {
        let state = DunningState::WrittenOff;
        let json = serde_json::to_string(&state).unwrap();
        assert_eq!(json, "\"written_off\"");
    }

    #[test]
    fn recon_exception_kind_serializes_as_snake_case() {
        let kind = ReconExceptionKind::UnmatchedPayment;
        let json = serde_json::to_string(&kind).unwrap();
        assert_eq!(json, "\"unmatched_payment\"");
    }

    #[test]
    fn payment_allocated_payload_has_allocation_lines() {
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
            allocated_at: Utc::now(),
        };
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("allocation_strategy"));
        assert!(json.contains("allocations"));
        assert!(json.contains("remaining_after_minor"));
    }

    // ============================================================================
    // Phase 23b tax event tests
    // ============================================================================

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
    fn tax_line_detail_serializes_correctly() {
        let detail = TaxLineDetail {
            line_id: "line-1".to_string(),
            tax_minor: 500,
            rate: 0.05,
            jurisdiction: "New York".to_string(),
            tax_type: "sales_tax".to_string(),
        };
        let json = serde_json::to_string(&detail).unwrap();
        assert!(json.contains("tax_minor"));
        assert!(json.contains("jurisdiction"));
        assert!(json.contains("New York"));
    }
}
