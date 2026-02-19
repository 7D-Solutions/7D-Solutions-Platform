//! Party event contracts v1.
//!
//! All events carry a full EventEnvelope with:
//! - schema_version: "1.0.0"
//! - source_module: "party"
//! - mutation_class: DATA_MUTATION or LIFECYCLE
//! - replay_safe: true

pub mod envelope;
pub mod party;

// ============================================================================
// Shared Constants
// ============================================================================

pub const PARTY_EVENT_SCHEMA_VERSION: &str = "1.0.0";
pub const MUTATION_CLASS_DATA_MUTATION: &str = "DATA_MUTATION";
pub const MUTATION_CLASS_LIFECYCLE: &str = "LIFECYCLE";

// ============================================================================
// Re-exports
// ============================================================================

pub use party::{
    build_party_created_envelope, build_party_deactivated_envelope,
    build_party_updated_envelope, PartyCreatedPayload, PartyDeactivatedPayload,
    PartyUpdatedPayload, EVENT_TYPE_PARTY_CREATED, EVENT_TYPE_PARTY_DEACTIVATED,
    EVENT_TYPE_PARTY_UPDATED,
};
