pub mod consumer;
pub mod contracts;
pub mod dlq;
pub mod envelope;
pub mod outbox;
pub mod publisher;

pub use consumer::{is_event_processed, mark_event_processed, process_event_idempotent};
pub use contracts::{
    AgingBuckets, ArAgingUpdatedPayload, CreditNoteIssuedPayload, InvoiceWrittenOffPayload,
    UsageCapturedPayload, UsageInvoicedPayload,
    AR_EVENT_SCHEMA_VERSION, EVENT_TYPE_AR_AGING_UPDATED, EVENT_TYPE_CREDIT_NOTE_ISSUED,
    EVENT_TYPE_INVOICE_WRITTEN_OFF, EVENT_TYPE_USAGE_CAPTURED, EVENT_TYPE_USAGE_INVOICED,
    MUTATION_CLASS_DATA_MUTATION, MUTATION_CLASS_REVERSAL,
    build_ar_aging_updated_envelope, build_credit_note_issued_envelope,
    build_invoice_written_off_envelope, build_usage_captured_envelope,
    build_usage_invoiced_envelope,
};
pub use envelope::EventEnvelope;
pub use outbox::enqueue_event;
pub use publisher::run_publisher_task;
