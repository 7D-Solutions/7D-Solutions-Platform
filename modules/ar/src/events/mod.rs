pub mod consumer;
pub mod contracts;
pub mod dlq;
pub mod envelope;
pub mod outbox;
pub mod publisher;

pub use consumer::{is_event_processed, mark_event_processed, process_event_idempotent};
pub use contracts::{
    build_ar_aging_updated_envelope,
    build_credit_memo_approved_envelope,
    build_credit_memo_created_envelope,
    build_credit_note_issued_envelope,
    build_dunning_state_changed_envelope,
    build_invoice_attempting_envelope,
    build_invoice_failed_final_envelope,
    build_invoice_opened_envelope,
    build_invoice_paid_envelope,
    build_invoice_settled_fx_envelope,
    build_invoice_suspended_envelope,
    build_invoice_void_envelope,
    build_invoice_written_off_envelope,
    build_milestone_invoice_created_envelope,
    build_payment_allocated_envelope,
    build_recon_exception_raised_envelope,
    build_recon_match_applied_envelope,
    build_recon_run_started_envelope,
    build_tax_committed_envelope,
    build_tax_quoted_envelope,
    build_tax_voided_envelope,
    build_usage_captured_envelope,
    build_usage_invoiced_envelope,
    // Phase 21 types
    AgingBuckets,
    // Phase 22 types
    AllocationLine,
    ArAgingUpdatedPayload,
    CreditMemoApprovedPayload,
    CreditMemoCreatedPayload,
    CreditNoteIssuedPayload,
    DunningState,
    DunningStateChangedPayload,
    // Phase 51 invoice lifecycle types
    InvoiceLifecyclePayload,
    // Phase 23a FX settlement event types
    InvoiceSettledFxPayload,
    InvoiceSuspendedPayload,
    InvoiceWrittenOffPayload,
    // Phase 63 progress billing types
    MilestoneInvoiceCreatedPayload,
    PaymentAllocatedPayload,
    ReconExceptionKind,
    ReconExceptionRaisedPayload,
    ReconMatchAppliedPayload,
    ReconRunStartedPayload,
    TaxCommittedPayload,
    // Phase 23b tax event types
    TaxLineDetail,
    TaxQuotedPayload,
    TaxVoidedPayload,
    UsageCapturedPayload,
    UsageInvoicedPayload,
    AR_EVENT_SCHEMA_VERSION,
    EVENT_TYPE_AR_AGING_UPDATED,
    EVENT_TYPE_CREDIT_MEMO_APPROVED,
    EVENT_TYPE_CREDIT_MEMO_CREATED,
    EVENT_TYPE_CREDIT_NOTE_ISSUED,
    EVENT_TYPE_DUNNING_STATE_CHANGED,
    EVENT_TYPE_INVOICE_ATTEMPTING,
    EVENT_TYPE_INVOICE_FAILED_FINAL,
    EVENT_TYPE_INVOICE_OPENED,
    EVENT_TYPE_INVOICE_PAID,
    EVENT_TYPE_INVOICE_SETTLED_FX,
    EVENT_TYPE_INVOICE_SUSPENDED,
    EVENT_TYPE_INVOICE_VOID,
    EVENT_TYPE_INVOICE_WRITTEN_OFF,
    EVENT_TYPE_MILESTONE_INVOICE_CREATED,
    EVENT_TYPE_PAYMENT_ALLOCATED,
    EVENT_TYPE_RECON_EXCEPTION_RAISED,
    EVENT_TYPE_RECON_MATCH_APPLIED,
    EVENT_TYPE_RECON_RUN_STARTED,
    EVENT_TYPE_TAX_COMMITTED,
    EVENT_TYPE_TAX_QUOTED,
    EVENT_TYPE_TAX_VOIDED,
    EVENT_TYPE_USAGE_CAPTURED,
    EVENT_TYPE_USAGE_INVOICED,
    MUTATION_CLASS_DATA_MUTATION,
    MUTATION_CLASS_LIFECYCLE,
    MUTATION_CLASS_REVERSAL,
};
pub use envelope::EventEnvelope;
#[allow(deprecated)]
pub use outbox::enqueue_event;
pub use publisher::run_publisher_task;
