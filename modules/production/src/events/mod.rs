use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const PRODUCTION_EVENT_SCHEMA_VERSION: &str = "1.0.0";
pub const MUTATION_CLASS_DATA_MUTATION: &str = "DATA_MUTATION";

#[derive(Debug, Clone, Copy)]
pub enum ProductionEventType {
    WorkOrderCreated,
    WorkOrderReleased,
    WorkOrderClosed,
    ComponentIssued,
    OperationStarted,
    OperationCompleted,
    FgReceived,
    WorkcenterCreated,
    WorkcenterUpdated,
    WorkcenterDeactivated,
}

impl ProductionEventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::WorkOrderCreated => "production.work_order_created",
            Self::WorkOrderReleased => "production.work_order_released",
            Self::WorkOrderClosed => "production.work_order_closed",
            Self::ComponentIssued => "production.component_issued",
            Self::OperationStarted => "production.operation_started",
            Self::OperationCompleted => "production.operation_completed",
            Self::FgReceived => "production.fg_received",
            Self::WorkcenterCreated => "production.workcenter_created",
            Self::WorkcenterUpdated => "production.workcenter_updated",
            Self::WorkcenterDeactivated => "production.workcenter_deactivated",
        }
    }
}

// ============================================================================
// Workcenter event payloads
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
pub struct WorkcenterCreatedPayload {
    pub workcenter_id: Uuid,
    pub tenant_id: String,
    pub code: String,
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WorkcenterUpdatedPayload {
    pub workcenter_id: Uuid,
    pub tenant_id: String,
    pub code: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WorkcenterDeactivatedPayload {
    pub workcenter_id: Uuid,
    pub tenant_id: String,
    pub code: String,
}

// ============================================================================
// Envelope builders
// ============================================================================

fn create_production_envelope<T>(
    tenant_id: String,
    event_type: String,
    correlation_id: String,
    causation_id: Option<String>,
    payload: T,
) -> event_bus::EventEnvelope<T> {
    event_bus::EventEnvelope::new(
        tenant_id,
        "production".to_string(),
        event_type,
        payload,
    )
    .with_source_version(env!("CARGO_PKG_VERSION").to_string())
    .with_trace_id(Some(correlation_id.clone()))
    .with_correlation_id(Some(correlation_id))
    .with_causation_id(causation_id)
    .with_mutation_class(Some(MUTATION_CLASS_DATA_MUTATION.to_string()))
    .with_replay_safe(true)
}

pub fn build_workcenter_created_envelope(
    workcenter_id: Uuid,
    tenant_id: String,
    code: String,
    name: String,
    correlation_id: String,
    causation_id: Option<String>,
) -> event_bus::EventEnvelope<WorkcenterCreatedPayload> {
    create_production_envelope(
        tenant_id.clone(),
        ProductionEventType::WorkcenterCreated.as_str().to_string(),
        correlation_id,
        causation_id,
        WorkcenterCreatedPayload {
            workcenter_id,
            tenant_id,
            code,
            name,
        },
    )
}

pub fn build_workcenter_updated_envelope(
    workcenter_id: Uuid,
    tenant_id: String,
    code: String,
    correlation_id: String,
    causation_id: Option<String>,
) -> event_bus::EventEnvelope<WorkcenterUpdatedPayload> {
    create_production_envelope(
        tenant_id.clone(),
        ProductionEventType::WorkcenterUpdated.as_str().to_string(),
        correlation_id,
        causation_id,
        WorkcenterUpdatedPayload {
            workcenter_id,
            tenant_id,
            code,
        },
    )
}

pub fn build_workcenter_deactivated_envelope(
    workcenter_id: Uuid,
    tenant_id: String,
    code: String,
    correlation_id: String,
    causation_id: Option<String>,
) -> event_bus::EventEnvelope<WorkcenterDeactivatedPayload> {
    create_production_envelope(
        tenant_id.clone(),
        ProductionEventType::WorkcenterDeactivated.as_str().to_string(),
        correlation_id,
        causation_id,
        WorkcenterDeactivatedPayload {
            workcenter_id,
            tenant_id,
            code,
        },
    )
}
