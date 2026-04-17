//! Event payload stubs for customer-complaints v1.
//! Full event emission wires up in bd-4l79e.1 (HTTP routes) and bd-4l79e.2 (consumers + sweep).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::envelope::{create_cc_envelope, EventEnvelope};
use super::{MUTATION_DATA, MUTATION_LIFECYCLE};

pub const EVENT_COMPLAINT_RECEIVED: &str = "customer_complaints.complaint_received";
pub const EVENT_COMPLAINT_TRIAGED: &str = "customer_complaints.complaint_triaged";
pub const EVENT_COMPLAINT_STATUS_CHANGED: &str = "customer_complaints.status_changed";
pub const EVENT_COMPLAINT_ASSIGNED: &str = "customer_complaints.assigned";
pub const EVENT_COMPLAINT_CUSTOMER_COMMUNICATED: &str = "customer_complaints.customer_communicated";
pub const EVENT_COMPLAINT_RESOLVED: &str = "customer_complaints.resolved";
pub const EVENT_COMPLAINT_CLOSED: &str = "customer_complaints.closed";
pub const EVENT_COMPLAINT_OVERDUE: &str = "customer_complaints.complaint_overdue";

// ── complaint.received ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplaintReceivedPayload {
    pub complaint_id: Uuid,
    pub complaint_number: String,
    pub tenant_id: String,
    pub party_id: Uuid,
    pub source: String,
    pub severity: Option<String>,
    pub category_code: Option<String>,
    pub source_entity_type: Option<String>,
    pub source_entity_id: Option<Uuid>,
    pub received_at: DateTime<Utc>,
}

pub fn build_complaint_received_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ComplaintReceivedPayload,
) -> EventEnvelope<ComplaintReceivedPayload> {
    create_cc_envelope(event_id, tenant_id, EVENT_COMPLAINT_RECEIVED.to_string(), correlation_id, causation_id, MUTATION_DATA.to_string(), payload)
}

// ── complaint.triaged ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplaintTriagedPayload {
    pub complaint_id: Uuid,
    pub tenant_id: String,
    pub assigned_to: String,
    pub category_code: String,
    pub severity: String,
    pub triaged_at: DateTime<Utc>,
}

pub fn build_complaint_triaged_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ComplaintTriagedPayload,
) -> EventEnvelope<ComplaintTriagedPayload> {
    create_cc_envelope(event_id, tenant_id, EVENT_COMPLAINT_TRIAGED.to_string(), correlation_id, causation_id, MUTATION_LIFECYCLE.to_string(), payload)
}

// ── complaint.status_changed ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplaintStatusChangedPayload {
    pub complaint_id: Uuid,
    pub tenant_id: String,
    pub from_status: String,
    pub to_status: String,
    pub transitioned_by: String,
    pub transitioned_at: DateTime<Utc>,
}

pub fn build_complaint_status_changed_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ComplaintStatusChangedPayload,
) -> EventEnvelope<ComplaintStatusChangedPayload> {
    create_cc_envelope(event_id, tenant_id, EVENT_COMPLAINT_STATUS_CHANGED.to_string(), correlation_id, causation_id, MUTATION_LIFECYCLE.to_string(), payload)
}

// ── complaint.assigned ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplaintAssignedPayload {
    pub complaint_id: Uuid,
    pub tenant_id: String,
    pub from_user: Option<String>,
    pub to_user: String,
    pub assigned_by: String,
    pub assigned_at: DateTime<Utc>,
}

pub fn build_complaint_assigned_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ComplaintAssignedPayload,
) -> EventEnvelope<ComplaintAssignedPayload> {
    create_cc_envelope(event_id, tenant_id, EVENT_COMPLAINT_ASSIGNED.to_string(), correlation_id, causation_id, MUTATION_DATA.to_string(), payload)
}

// ── complaint.customer_communicated ──────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplaintCustomerCommunicatedPayload {
    pub complaint_id: Uuid,
    pub tenant_id: String,
    pub recorded_by: String,
    pub recorded_at: DateTime<Utc>,
}

pub fn build_complaint_customer_communicated_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ComplaintCustomerCommunicatedPayload,
) -> EventEnvelope<ComplaintCustomerCommunicatedPayload> {
    create_cc_envelope(event_id, tenant_id, EVENT_COMPLAINT_CUSTOMER_COMMUNICATED.to_string(), correlation_id, causation_id, MUTATION_DATA.to_string(), payload)
}

// ── complaint.resolved ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplaintResolvedPayload {
    pub complaint_id: Uuid,
    pub tenant_id: String,
    pub customer_acceptance: String,
    pub resolved_by: String,
    pub resolved_at: DateTime<Utc>,
}

pub fn build_complaint_resolved_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ComplaintResolvedPayload,
) -> EventEnvelope<ComplaintResolvedPayload> {
    create_cc_envelope(event_id, tenant_id, EVENT_COMPLAINT_RESOLVED.to_string(), correlation_id, causation_id, MUTATION_LIFECYCLE.to_string(), payload)
}

// ── complaint.closed ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplaintClosedPayload {
    pub complaint_id: Uuid,
    pub tenant_id: String,
    pub outcome: String,
    pub closed_at: DateTime<Utc>,
}

pub fn build_complaint_closed_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ComplaintClosedPayload,
) -> EventEnvelope<ComplaintClosedPayload> {
    create_cc_envelope(event_id, tenant_id, EVENT_COMPLAINT_CLOSED.to_string(), correlation_id, causation_id, MUTATION_LIFECYCLE.to_string(), payload)
}

// ── complaint.overdue ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplaintOverduePayload {
    pub complaint_id: Uuid,
    pub tenant_id: String,
    pub assigned_to: Option<String>,
    pub due_date: DateTime<Utc>,
    pub days_overdue: i64,
    pub severity: Option<String>,
}

pub fn build_complaint_overdue_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: ComplaintOverduePayload,
) -> EventEnvelope<ComplaintOverduePayload> {
    create_cc_envelope(event_id, tenant_id, EVENT_COMPLAINT_OVERDUE.to_string(), correlation_id, causation_id, MUTATION_LIFECYCLE.to_string(), payload)
}
