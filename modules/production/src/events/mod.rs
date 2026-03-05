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
    RoutingCreated,
    RoutingUpdated,
    RoutingReleased,
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
            Self::RoutingCreated => "production.routing_created",
            Self::RoutingUpdated => "production.routing_updated",
            Self::RoutingReleased => "production.routing_released",
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

// ============================================================================
// Work order event payloads
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
pub struct WorkOrderCreatedPayload {
    pub work_order_id: Uuid,
    pub tenant_id: String,
    pub order_number: String,
    pub item_id: Uuid,
    pub bom_revision_id: Uuid,
    pub planned_quantity: i32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WorkOrderReleasedPayload {
    pub work_order_id: Uuid,
    pub tenant_id: String,
    pub order_number: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WorkOrderClosedPayload {
    pub work_order_id: Uuid,
    pub tenant_id: String,
    pub order_number: String,
}

// ============================================================================
// Work order envelope builders
// ============================================================================

pub fn build_work_order_created_envelope(
    work_order_id: Uuid,
    tenant_id: String,
    order_number: String,
    item_id: Uuid,
    bom_revision_id: Uuid,
    planned_quantity: i32,
    correlation_id: String,
    causation_id: Option<String>,
) -> event_bus::EventEnvelope<WorkOrderCreatedPayload> {
    create_production_envelope(
        tenant_id.clone(),
        ProductionEventType::WorkOrderCreated.as_str().to_string(),
        correlation_id,
        causation_id,
        WorkOrderCreatedPayload {
            work_order_id,
            tenant_id,
            order_number,
            item_id,
            bom_revision_id,
            planned_quantity,
        },
    )
}

pub fn build_work_order_released_envelope(
    work_order_id: Uuid,
    tenant_id: String,
    order_number: String,
    correlation_id: String,
    causation_id: Option<String>,
) -> event_bus::EventEnvelope<WorkOrderReleasedPayload> {
    create_production_envelope(
        tenant_id.clone(),
        ProductionEventType::WorkOrderReleased.as_str().to_string(),
        correlation_id,
        causation_id,
        WorkOrderReleasedPayload {
            work_order_id,
            tenant_id,
            order_number,
        },
    )
}

pub fn build_work_order_closed_envelope(
    work_order_id: Uuid,
    tenant_id: String,
    order_number: String,
    correlation_id: String,
    causation_id: Option<String>,
) -> event_bus::EventEnvelope<WorkOrderClosedPayload> {
    create_production_envelope(
        tenant_id.clone(),
        ProductionEventType::WorkOrderClosed.as_str().to_string(),
        correlation_id,
        causation_id,
        WorkOrderClosedPayload {
            work_order_id,
            tenant_id,
            order_number,
        },
    )
}

// ============================================================================
// Routing event payloads
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
pub struct RoutingCreatedPayload {
    pub routing_template_id: Uuid,
    pub tenant_id: String,
    pub name: String,
    pub revision: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RoutingUpdatedPayload {
    pub routing_template_id: Uuid,
    pub tenant_id: String,
    pub name: String,
    pub revision: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RoutingReleasedPayload {
    pub routing_template_id: Uuid,
    pub tenant_id: String,
    pub name: String,
    pub revision: String,
}

// ============================================================================
// Routing envelope builders
// ============================================================================

pub fn build_routing_created_envelope(
    routing_template_id: Uuid,
    tenant_id: String,
    name: String,
    revision: String,
    correlation_id: String,
    causation_id: Option<String>,
) -> event_bus::EventEnvelope<RoutingCreatedPayload> {
    create_production_envelope(
        tenant_id.clone(),
        ProductionEventType::RoutingCreated.as_str().to_string(),
        correlation_id,
        causation_id,
        RoutingCreatedPayload {
            routing_template_id,
            tenant_id,
            name,
            revision,
        },
    )
}

pub fn build_routing_updated_envelope(
    routing_template_id: Uuid,
    tenant_id: String,
    name: String,
    revision: String,
    correlation_id: String,
    causation_id: Option<String>,
) -> event_bus::EventEnvelope<RoutingUpdatedPayload> {
    create_production_envelope(
        tenant_id.clone(),
        ProductionEventType::RoutingUpdated.as_str().to_string(),
        correlation_id,
        causation_id,
        RoutingUpdatedPayload {
            routing_template_id,
            tenant_id,
            name,
            revision,
        },
    )
}

pub fn build_routing_released_envelope(
    routing_template_id: Uuid,
    tenant_id: String,
    name: String,
    revision: String,
    correlation_id: String,
    causation_id: Option<String>,
) -> event_bus::EventEnvelope<RoutingReleasedPayload> {
    create_production_envelope(
        tenant_id.clone(),
        ProductionEventType::RoutingReleased.as_str().to_string(),
        correlation_id,
        causation_id,
        RoutingReleasedPayload {
            routing_template_id,
            tenant_id,
            name,
            revision,
        },
    )
}
