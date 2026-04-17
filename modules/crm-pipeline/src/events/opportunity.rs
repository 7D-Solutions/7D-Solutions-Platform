//! CRM Pipeline opportunity events.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{CRM_SCHEMA_VERSION, MUTATION_CLASS_DATA_MUTATION, MUTATION_CLASS_LIFECYCLE};
use crate::events::envelope::{create_crm_envelope, EventEnvelope};

pub const EVENT_TYPE_OPPORTUNITY_CREATED: &str = "crm_pipeline.opportunity_created";
pub const EVENT_TYPE_OPPORTUNITY_STAGE_ADVANCED: &str = "crm_pipeline.opportunity_stage_advanced";
pub const EVENT_TYPE_OPPORTUNITY_CLOSED_WON: &str = "crm_pipeline.opportunity_closed_won";
pub const EVENT_TYPE_OPPORTUNITY_CLOSED_LOST: &str = "crm_pipeline.opportunity_closed_lost";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpportunityCreatedPayload {
    pub opportunity_id: Uuid,
    pub tenant_id: String,
    pub opp_number: String,
    pub party_id: Uuid,
    pub stage_code: String,
    pub estimated_value_cents: Option<i64>,
    pub created_at: DateTime<Utc>,
}

pub fn build_opportunity_created_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    payload: OpportunityCreatedPayload,
) -> EventEnvelope<OpportunityCreatedPayload> {
    create_crm_envelope(
        event_id, tenant_id, EVENT_TYPE_OPPORTUNITY_CREATED.to_string(),
        correlation_id, None, MUTATION_CLASS_DATA_MUTATION.to_string(), payload,
    )
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpportunityStageAdvancedPayload {
    pub opportunity_id: Uuid,
    pub tenant_id: String,
    pub from_stage_code: String,
    pub to_stage_code: String,
    pub probability_pct: i32,
    pub days_in_previous_stage: Option<i32>,
}

pub fn build_opportunity_stage_advanced_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    payload: OpportunityStageAdvancedPayload,
) -> EventEnvelope<OpportunityStageAdvancedPayload> {
    create_crm_envelope(
        event_id, tenant_id, EVENT_TYPE_OPPORTUNITY_STAGE_ADVANCED.to_string(),
        correlation_id, None, MUTATION_CLASS_LIFECYCLE.to_string(), payload,
    )
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpportunityClosedWonPayload {
    pub opportunity_id: Uuid,
    pub tenant_id: String,
    pub party_id: Uuid,
    pub actual_close_date: NaiveDate,
    pub estimated_value_cents: Option<i64>,
    pub sales_order_id: Option<Uuid>,
}

pub fn build_opportunity_closed_won_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    payload: OpportunityClosedWonPayload,
) -> EventEnvelope<OpportunityClosedWonPayload> {
    create_crm_envelope(
        event_id, tenant_id, EVENT_TYPE_OPPORTUNITY_CLOSED_WON.to_string(),
        correlation_id, None, MUTATION_CLASS_LIFECYCLE.to_string(), payload,
    )
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpportunityClosedLostPayload {
    pub opportunity_id: Uuid,
    pub tenant_id: String,
    pub party_id: Uuid,
    pub actual_close_date: NaiveDate,
    pub close_reason: String,
    pub competitor: Option<String>,
}

pub fn build_opportunity_closed_lost_envelope(
    event_id: Uuid,
    tenant_id: String,
    correlation_id: String,
    payload: OpportunityClosedLostPayload,
) -> EventEnvelope<OpportunityClosedLostPayload> {
    create_crm_envelope(
        event_id, tenant_id, EVENT_TYPE_OPPORTUNITY_CLOSED_LOST.to_string(),
        correlation_id, None, MUTATION_CLASS_LIFECYCLE.to_string(), payload,
    )
}
