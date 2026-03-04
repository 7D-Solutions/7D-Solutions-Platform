//! Party event contracts v1.
//!
//! All events carry a full EventEnvelope with:
//! - schema_version: "1.0.0"
//! - source_module: "party"
//! - mutation_class: DATA_MUTATION or LIFECYCLE
//! - replay_safe: true

pub mod contact;
pub mod envelope;
pub mod party;
pub mod vendor;

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
    build_party_created_envelope, build_party_deactivated_envelope, build_party_updated_envelope,
    PartyCreatedPayload, PartyDeactivatedPayload, PartyUpdatedPayload, EVENT_TYPE_PARTY_CREATED,
    EVENT_TYPE_PARTY_DEACTIVATED, EVENT_TYPE_PARTY_UPDATED,
};

pub use contact::{
    build_contact_created_envelope, build_contact_deactivated_envelope,
    build_contact_primary_set_envelope, build_contact_updated_envelope,
    build_tags_updated_envelope, ContactDeactivatedPayload, ContactPayload,
    ContactPrimarySetPayload, TagsUpdatedPayload, EVENT_TYPE_CONTACT_CREATED,
    EVENT_TYPE_CONTACT_DEACTIVATED, EVENT_TYPE_CONTACT_PRIMARY_SET, EVENT_TYPE_CONTACT_UPDATED,
    EVENT_TYPE_TAGS_UPDATED,
};

pub use vendor::{
    build_contact_role_created_envelope, build_contact_role_updated_envelope,
    build_credit_terms_created_envelope, build_credit_terms_updated_envelope,
    build_scorecard_created_envelope, build_scorecard_updated_envelope,
    build_vendor_qualification_created_envelope, build_vendor_qualification_updated_envelope,
    ContactRolePayload, CreditTermsPayload, ScorecardPayload, VendorQualificationPayload,
    EVENT_TYPE_CONTACT_ROLE_CREATED, EVENT_TYPE_CONTACT_ROLE_UPDATED,
    EVENT_TYPE_CREDIT_TERMS_CREATED, EVENT_TYPE_CREDIT_TERMS_UPDATED,
    EVENT_TYPE_SCORECARD_CREATED, EVENT_TYPE_SCORECARD_UPDATED,
    EVENT_TYPE_VENDOR_QUALIFICATION_CREATED, EVENT_TYPE_VENDOR_QUALIFICATION_UPDATED,
};
