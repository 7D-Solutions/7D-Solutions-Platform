//! Vendor event contracts: qualification, credit_terms, contact_role, scorecard.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::envelope::{create_party_envelope, EventEnvelope};
use super::{MUTATION_CLASS_DATA_MUTATION, PARTY_EVENT_SCHEMA_VERSION};

// ============================================================================
// Event Type Constants
// ============================================================================

pub const EVENT_TYPE_VENDOR_QUALIFICATION_CREATED: &str = "party.vendor_qualification.created";
pub const EVENT_TYPE_VENDOR_QUALIFICATION_UPDATED: &str = "party.vendor_qualification.updated";
pub const EVENT_TYPE_CREDIT_TERMS_CREATED: &str = "party.credit_terms.created";
pub const EVENT_TYPE_CREDIT_TERMS_UPDATED: &str = "party.credit_terms.updated";
pub const EVENT_TYPE_CONTACT_ROLE_CREATED: &str = "party.contact_role.created";
pub const EVENT_TYPE_CONTACT_ROLE_UPDATED: &str = "party.contact_role.updated";
pub const EVENT_TYPE_SCORECARD_CREATED: &str = "party.scorecard.created";
pub const EVENT_TYPE_SCORECARD_UPDATED: &str = "party.scorecard.updated";

// ============================================================================
// Payload: vendor_qualification
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VendorQualificationPayload {
    pub qualification_id: Uuid,
    pub party_id: Uuid,
    pub app_id: String,
    pub qualification_status: String,
    pub certification_ref: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
}

pub fn build_vendor_qualification_created_envelope(
    event_id: Uuid,
    app_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: VendorQualificationPayload,
) -> EventEnvelope<VendorQualificationPayload> {
    create_party_envelope(
        event_id,
        app_id,
        EVENT_TYPE_VENDOR_QUALIFICATION_CREATED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(PARTY_EVENT_SCHEMA_VERSION.to_string())
}

pub fn build_vendor_qualification_updated_envelope(
    event_id: Uuid,
    app_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: VendorQualificationPayload,
) -> EventEnvelope<VendorQualificationPayload> {
    create_party_envelope(
        event_id,
        app_id,
        EVENT_TYPE_VENDOR_QUALIFICATION_UPDATED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(PARTY_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Payload: credit_terms
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreditTermsPayload {
    pub credit_terms_id: Uuid,
    pub party_id: Uuid,
    pub app_id: String,
    pub payment_terms: String,
    pub credit_limit_cents: Option<i64>,
    pub effective_from: NaiveDate,
}

pub fn build_credit_terms_created_envelope(
    event_id: Uuid,
    app_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: CreditTermsPayload,
) -> EventEnvelope<CreditTermsPayload> {
    create_party_envelope(
        event_id,
        app_id,
        EVENT_TYPE_CREDIT_TERMS_CREATED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(PARTY_EVENT_SCHEMA_VERSION.to_string())
}

pub fn build_credit_terms_updated_envelope(
    event_id: Uuid,
    app_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: CreditTermsPayload,
) -> EventEnvelope<CreditTermsPayload> {
    create_party_envelope(
        event_id,
        app_id,
        EVENT_TYPE_CREDIT_TERMS_UPDATED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(PARTY_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Payload: contact_role
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContactRolePayload {
    pub contact_role_id: Uuid,
    pub party_id: Uuid,
    pub contact_id: Uuid,
    pub app_id: String,
    pub role_type: String,
    pub is_primary: bool,
}

pub fn build_contact_role_created_envelope(
    event_id: Uuid,
    app_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ContactRolePayload,
) -> EventEnvelope<ContactRolePayload> {
    create_party_envelope(
        event_id,
        app_id,
        EVENT_TYPE_CONTACT_ROLE_CREATED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(PARTY_EVENT_SCHEMA_VERSION.to_string())
}

pub fn build_contact_role_updated_envelope(
    event_id: Uuid,
    app_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ContactRolePayload,
) -> EventEnvelope<ContactRolePayload> {
    create_party_envelope(
        event_id,
        app_id,
        EVENT_TYPE_CONTACT_ROLE_UPDATED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(PARTY_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Payload: scorecard
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScorecardPayload {
    pub scorecard_id: Uuid,
    pub party_id: Uuid,
    pub app_id: String,
    pub metric_name: String,
    pub score: f64,
    pub review_date: NaiveDate,
}

pub fn build_scorecard_created_envelope(
    event_id: Uuid,
    app_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ScorecardPayload,
) -> EventEnvelope<ScorecardPayload> {
    create_party_envelope(
        event_id,
        app_id,
        EVENT_TYPE_SCORECARD_CREATED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(PARTY_EVENT_SCHEMA_VERSION.to_string())
}

pub fn build_scorecard_updated_envelope(
    event_id: Uuid,
    app_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ScorecardPayload,
) -> EventEnvelope<ScorecardPayload> {
    create_party_envelope(
        event_id,
        app_id,
        EVENT_TYPE_SCORECARD_UPDATED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(PARTY_EVENT_SCHEMA_VERSION.to_string())
}
