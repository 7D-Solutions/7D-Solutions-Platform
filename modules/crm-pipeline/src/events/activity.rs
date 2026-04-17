//! CRM Pipeline activity events.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{MUTATION_CLASS_DATA_MUTATION, MUTATION_CLASS_LIFECYCLE};
use crate::events::envelope::{create_crm_envelope, EventEnvelope};

pub const EVENT_TYPE_ACTIVITY_LOGGED: &str = "crm_pipeline.activity_logged";
pub const EVENT_TYPE_ACTIVITY_COMPLETED: &str = "crm_pipeline.activity_completed";
pub const EVENT_TYPE_ACTIVITY_OVERDUE: &str = "crm_pipeline.activity_overdue";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityLoggedPayload {
    pub activity_id: Uuid,
    pub tenant_id: String,
    pub activity_type_code: String,
    pub entity_type: String,
    pub entity_id: Uuid,
    pub assigned_to: Option<String>,
}

pub fn build_activity_logged_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    payload: ActivityLoggedPayload,
) -> EventEnvelope<ActivityLoggedPayload> {
    create_crm_envelope(
        event_id, tenant_id, EVENT_TYPE_ACTIVITY_LOGGED.to_string(),
        correlation_id, None, MUTATION_CLASS_DATA_MUTATION.to_string(), payload,
    )
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityCompletedPayload {
    pub activity_id: Uuid,
    pub tenant_id: String,
    pub completed_at: DateTime<Utc>,
    pub completed_by: String,
}

pub fn build_activity_completed_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    payload: ActivityCompletedPayload,
) -> EventEnvelope<ActivityCompletedPayload> {
    create_crm_envelope(
        event_id, tenant_id, EVENT_TYPE_ACTIVITY_COMPLETED.to_string(),
        correlation_id, None, MUTATION_CLASS_LIFECYCLE.to_string(), payload,
    )
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityOverduePayload {
    pub activity_id: Uuid,
    pub tenant_id: String,
    pub assigned_to: Option<String>,
    pub due_date: NaiveDate,
    pub days_overdue: i64,
}

pub fn build_activity_overdue_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    payload: ActivityOverduePayload,
) -> EventEnvelope<ActivityOverduePayload> {
    create_crm_envelope(
        event_id, tenant_id, EVENT_TYPE_ACTIVITY_OVERDUE.to_string(),
        correlation_id, None, MUTATION_CLASS_LIFECYCLE.to_string(), payload,
    )
}
