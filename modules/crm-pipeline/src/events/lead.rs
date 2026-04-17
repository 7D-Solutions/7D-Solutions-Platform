//! CRM Pipeline lead events.
//!
//! Event type strings: crm_pipeline.lead_created, crm_pipeline.lead_status_changed,
//! crm_pipeline.lead_converted — dot notation, no .v1 suffix in code.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{CRM_SCHEMA_VERSION, MUTATION_CLASS_DATA_MUTATION, MUTATION_CLASS_LIFECYCLE};
use crate::events::envelope::{create_crm_envelope, EventEnvelope};

pub const EVENT_TYPE_LEAD_CREATED: &str = "crm_pipeline.lead_created";
pub const EVENT_TYPE_LEAD_STATUS_CHANGED: &str = "crm_pipeline.lead_status_changed";
pub const EVENT_TYPE_LEAD_CONVERTED: &str = "crm_pipeline.lead_converted";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeadCreatedPayload {
    pub lead_id: Uuid,
    pub tenant_id: String,
    pub lead_number: String,
    pub source: String,
    pub company_name: String,
    pub estimated_value_cents: Option<i64>,
    pub created_at: DateTime<Utc>,
}

pub fn build_lead_created_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    payload: LeadCreatedPayload,
) -> EventEnvelope<LeadCreatedPayload> {
    create_crm_envelope(
        event_id, tenant_id, EVENT_TYPE_LEAD_CREATED.to_string(),
        correlation_id, None, MUTATION_CLASS_DATA_MUTATION.to_string(), payload,
    )
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeadStatusChangedPayload {
    pub lead_id: Uuid,
    pub tenant_id: String,
    pub lead_number: String,
    pub from_status: String,
    pub to_status: String,
    pub changed_by: String,
    pub changed_at: DateTime<Utc>,
}

pub fn build_lead_status_changed_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    payload: LeadStatusChangedPayload,
) -> EventEnvelope<LeadStatusChangedPayload> {
    create_crm_envelope(
        event_id, tenant_id, EVENT_TYPE_LEAD_STATUS_CHANGED.to_string(),
        correlation_id, None, MUTATION_CLASS_LIFECYCLE.to_string(), payload,
    )
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeadConvertedPayload {
    pub lead_id: Uuid,
    pub tenant_id: String,
    pub opportunity_id: Option<Uuid>,
    pub party_id: Uuid,
    pub party_contact_id: Option<Uuid>,
    pub converted_at: DateTime<Utc>,
}

pub fn build_lead_converted_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    payload: LeadConvertedPayload,
) -> EventEnvelope<LeadConvertedPayload> {
    create_crm_envelope(
        event_id, tenant_id, EVENT_TYPE_LEAD_CONVERTED.to_string(),
        correlation_id, None, MUTATION_CLASS_LIFECYCLE.to_string(), payload,
    )
}
