//! Contact event contracts: contact.created, contact.updated, contact.deactivated,
//! contact.primary_set, tags.updated.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::envelope::{create_party_envelope, EventEnvelope};
use super::{MUTATION_CLASS_DATA_MUTATION, MUTATION_CLASS_LIFECYCLE, PARTY_EVENT_SCHEMA_VERSION};

// ============================================================================
// Event Type Constants
// ============================================================================

pub const EVENT_TYPE_CONTACT_CREATED: &str = "party.events.contact.created";
pub const EVENT_TYPE_CONTACT_UPDATED: &str = "party.events.contact.updated";
pub const EVENT_TYPE_CONTACT_DEACTIVATED: &str = "party.events.contact.deactivated";
pub const EVENT_TYPE_CONTACT_PRIMARY_SET: &str = "party.events.contact.primary_set";
pub const EVENT_TYPE_TAGS_UPDATED: &str = "party.events.tags.updated";

// ============================================================================
// Payload: contact.created / contact.updated
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContactPayload {
    pub contact_id: Uuid,
    pub party_id: Uuid,
    pub app_id: String,
    pub first_name: String,
    pub last_name: String,
    pub email: Option<String>,
    pub role: Option<String>,
    pub is_primary: bool,
}

pub fn build_contact_created_envelope(
    event_id: Uuid,
    app_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ContactPayload,
) -> EventEnvelope<ContactPayload> {
    create_party_envelope(
        event_id,
        app_id,
        EVENT_TYPE_CONTACT_CREATED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(PARTY_EVENT_SCHEMA_VERSION.to_string())
}

pub fn build_contact_updated_envelope(
    event_id: Uuid,
    app_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ContactPayload,
) -> EventEnvelope<ContactPayload> {
    create_party_envelope(
        event_id,
        app_id,
        EVENT_TYPE_CONTACT_UPDATED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(PARTY_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Payload: contact.deactivated
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContactDeactivatedPayload {
    pub contact_id: Uuid,
    pub party_id: Uuid,
    pub app_id: String,
    pub deactivated_at: DateTime<Utc>,
}

pub fn build_contact_deactivated_envelope(
    event_id: Uuid,
    app_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ContactDeactivatedPayload,
) -> EventEnvelope<ContactDeactivatedPayload> {
    create_party_envelope(
        event_id,
        app_id,
        EVENT_TYPE_CONTACT_DEACTIVATED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_LIFECYCLE.to_string(),
        payload,
    )
    .with_schema_version(PARTY_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Payload: contact.primary_set
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContactPrimarySetPayload {
    pub contact_id: Uuid,
    pub party_id: Uuid,
    pub app_id: String,
    pub role: String,
}

pub fn build_contact_primary_set_envelope(
    event_id: Uuid,
    app_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ContactPrimarySetPayload,
) -> EventEnvelope<ContactPrimarySetPayload> {
    create_party_envelope(
        event_id,
        app_id,
        EVENT_TYPE_CONTACT_PRIMARY_SET.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(PARTY_EVENT_SCHEMA_VERSION.to_string())
}

// ============================================================================
// Payload: tags.updated
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagsUpdatedPayload {
    pub party_id: Uuid,
    pub app_id: String,
    pub tags: Vec<String>,
}

pub fn build_tags_updated_envelope(
    event_id: Uuid,
    app_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: TagsUpdatedPayload,
) -> EventEnvelope<TagsUpdatedPayload> {
    create_party_envelope(
        event_id,
        app_id,
        EVENT_TYPE_TAGS_UPDATED.to_string(),
        correlation_id,
        causation_id,
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    )
    .with_schema_version(PARTY_EVENT_SCHEMA_VERSION.to_string())
}
