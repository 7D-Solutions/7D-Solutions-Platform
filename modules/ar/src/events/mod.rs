pub mod consumer;
pub mod contracts;
pub mod dlq;
pub mod envelope;
pub mod outbox;
pub mod publisher;

pub use consumer::{is_event_processed, mark_event_processed, process_event_idempotent};
pub use contracts::{
    // Phase 21 types
    AgingBuckets, ArAgingUpdatedPayload, CreditNoteIssuedPayload, InvoiceWrittenOffPayload,
    UsageCapturedPayload, UsageInvoicedPayload,
    AR_EVENT_SCHEMA_VERSION,
    EVENT_TYPE_AR_AGING_UPDATED, EVENT_TYPE_CREDIT_NOTE_ISSUED,
    EVENT_TYPE_INVOICE_WRITTEN_OFF, EVENT_TYPE_USAGE_CAPTURED, EVENT_TYPE_USAGE_INVOICED,
    MUTATION_CLASS_DATA_MUTATION, MUTATION_CLASS_REVERSAL, MUTATION_CLASS_LIFECYCLE,
    build_ar_aging_updated_envelope, build_credit_note_issued_envelope,
    build_invoice_written_off_envelope, build_usage_captured_envelope,
    build_usage_invoiced_envelope,
    // Phase 22 types
    AllocationLine, DunningState, DunningStateChangedPayload, InvoiceSuspendedPayload,
    PaymentAllocatedPayload, ReconExceptionKind, ReconExceptionRaisedPayload,
    ReconMatchAppliedPayload, ReconRunStartedPayload,
    EVENT_TYPE_DUNNING_STATE_CHANGED, EVENT_TYPE_INVOICE_SUSPENDED,
    EVENT_TYPE_PAYMENT_ALLOCATED, EVENT_TYPE_RECON_EXCEPTION_RAISED,
    EVENT_TYPE_RECON_MATCH_APPLIED, EVENT_TYPE_RECON_RUN_STARTED,
    build_dunning_state_changed_envelope, build_invoice_suspended_envelope,
    build_payment_allocated_envelope, build_recon_exception_raised_envelope,
    build_recon_match_applied_envelope, build_recon_run_started_envelope,
    // Phase 23b tax event types
    TaxLineDetail, TaxQuotedPayload, TaxCommittedPayload, TaxVoidedPayload,
    EVENT_TYPE_TAX_QUOTED, EVENT_TYPE_TAX_COMMITTED, EVENT_TYPE_TAX_VOIDED,
    build_tax_quoted_envelope, build_tax_committed_envelope, build_tax_voided_envelope,
};
pub use envelope::EventEnvelope;
pub use outbox::enqueue_event;
pub use publisher::run_publisher_task;
