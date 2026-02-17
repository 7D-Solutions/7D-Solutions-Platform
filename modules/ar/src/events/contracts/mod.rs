//! AR event type constants and payload structs
//!
//! Defines the canonical event contracts for AR's monetization events:
//! - ar.usage_captured       (metered usage recorded)
//! - ar.usage_invoiced       (usage billed on an invoice)
//! - ar.credit_note_issued   (credit note issued against an invoice)
//! - ar.invoice_written_off  (invoice written off as bad debt)
//! - ar.ar_aging_updated     (AR aging projection updated)
//! - ar.dunning_state_changed (dunning state machine transition)
//! - ar.invoice_suspended    (invoice suspended for non-payment)
//! - ar.recon_*              (reconciliation run, match, exception)
//! - ar.payment_allocated    (payment allocated to invoices)
//! - tax.quoted/committed/voided (tax lifecycle)
//! - ar.invoice_settled_fx   (FX settlement gain/loss)
//!
//! All events carry a full EventEnvelope with:
//! - schema_version: "1.0.0" (stable for this event version)
//! - mutation_class: per event (DATA_MUTATION or REVERSAL)
//! - correlation_id / causation_id: caller-supplied for tracing
//! - event_id: caller-supplied for idempotency (deterministic from business key)

mod aging_dunning;
mod credit_writeoff;
mod recon_allocation;
mod tax_fx;
mod usage;

// ============================================================================
// Shared Constants
// ============================================================================

/// Schema version for all AR monetization event payloads (v1)
pub const AR_EVENT_SCHEMA_VERSION: &str = "1.0.0";

/// DATA_MUTATION: creates or modifies a financial record
pub const MUTATION_CLASS_DATA_MUTATION: &str = "DATA_MUTATION";

/// REVERSAL: compensates for a prior DATA_MUTATION (write-off, void)
pub const MUTATION_CLASS_REVERSAL: &str = "REVERSAL";

/// LIFECYCLE: entity lifecycle transitions (dunning state changes, suspension)
pub const MUTATION_CLASS_LIFECYCLE: &str = "LIFECYCLE";

// ============================================================================
// Re-exports (preserves all existing import paths)
// ============================================================================

pub use aging_dunning::*;
pub use credit_writeoff::*;
pub use recon_allocation::*;
pub use tax_fx::*;
pub use usage::*;

#[cfg(test)]
mod tests {
    use super::*;

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
}
