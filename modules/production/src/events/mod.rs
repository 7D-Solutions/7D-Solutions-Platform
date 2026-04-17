use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const PRODUCTION_EVENT_SCHEMA_VERSION: &str = "1.0.0";
pub const MUTATION_CLASS_DATA_MUTATION: &str = "DATA_MUTATION";

#[derive(Debug, Clone, Copy)]
pub enum ProductionEventType {
    WorkOrderCreated,
    WorkOrderReleased,
    WorkOrderClosed,
    ComponentIssueRequested,
    ComponentIssued,
    OperationStarted,
    OperationCompleted,
    FgReceived,
    FgReceiptRequested,
    WorkcenterCreated,
    WorkcenterUpdated,
    WorkcenterDeactivated,
    RoutingCreated,
    RoutingUpdated,
    RoutingReleased,
    TimeEntryCreated,
    TimeEntryStopped,
    TimeEntryApproved,
    DowntimeStarted,
    DowntimeEnded,
}

impl ProductionEventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::WorkOrderCreated => "production.work_order_created",
            Self::WorkOrderReleased => "production.work_order_released",
            Self::WorkOrderClosed => "production.work_order_closed",
            Self::ComponentIssueRequested => "production.component_issue.requested",
            Self::ComponentIssued => "production.component_issued",
            Self::OperationStarted => "production.operation_started",
            Self::OperationCompleted => "production.operation_completed",
            Self::FgReceived => "production.fg_received",
            Self::FgReceiptRequested => "production.fg_receipt.requested",
            Self::WorkcenterCreated => "production.workcenter_created",
            Self::WorkcenterUpdated => "production.workcenter_updated",
            Self::WorkcenterDeactivated => "production.workcenter_deactivated",
            Self::RoutingCreated => "production.routing_created",
            Self::RoutingUpdated => "production.routing_updated",
            Self::RoutingReleased => "production.routing_released",
            Self::TimeEntryCreated => "production.time_entry_created",
            Self::TimeEntryStopped => "production.time_entry_stopped",
            Self::TimeEntryApproved => "production.time_entry_approved",
            Self::DowntimeStarted => "production.downtime.started",
            Self::DowntimeEnded => "production.downtime.ended",
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
    event_bus::EventEnvelope::new(tenant_id, "production".to_string(), event_type, payload)
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
        ProductionEventType::WorkcenterDeactivated
            .as_str()
            .to_string(),
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
    pub bom_revision_id: Option<Uuid>,
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
    bom_revision_id: Option<Uuid>,
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

// ============================================================================
// Operation event payloads
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
pub struct OperationStartedPayload {
    pub operation_id: Uuid,
    pub work_order_id: Uuid,
    pub tenant_id: String,
    pub operation_name: String,
    pub sequence_number: i32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OperationCompletedPayload {
    pub operation_id: Uuid,
    pub work_order_id: Uuid,
    pub tenant_id: String,
    pub operation_name: String,
    pub sequence_number: i32,
}

// ============================================================================
// Operation envelope builders
// ============================================================================

pub fn build_operation_started_envelope(
    operation_id: Uuid,
    work_order_id: Uuid,
    tenant_id: String,
    operation_name: String,
    sequence_number: i32,
    correlation_id: String,
    causation_id: Option<String>,
) -> event_bus::EventEnvelope<OperationStartedPayload> {
    create_production_envelope(
        tenant_id.clone(),
        ProductionEventType::OperationStarted.as_str().to_string(),
        correlation_id,
        causation_id,
        OperationStartedPayload {
            operation_id,
            work_order_id,
            tenant_id,
            operation_name,
            sequence_number,
        },
    )
}

pub fn build_operation_completed_envelope(
    operation_id: Uuid,
    work_order_id: Uuid,
    tenant_id: String,
    operation_name: String,
    sequence_number: i32,
    correlation_id: String,
    causation_id: Option<String>,
) -> event_bus::EventEnvelope<OperationCompletedPayload> {
    create_production_envelope(
        tenant_id.clone(),
        ProductionEventType::OperationCompleted.as_str().to_string(),
        correlation_id,
        causation_id,
        OperationCompletedPayload {
            operation_id,
            work_order_id,
            tenant_id,
            operation_name,
            sequence_number,
        },
    )
}

// ============================================================================
// Component issue event payloads
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentIssueItem {
    pub item_id: Uuid,
    pub warehouse_id: Uuid,
    pub quantity: i64,
    pub currency: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentIssueRequestedPayload {
    pub work_order_id: Uuid,
    pub tenant_id: String,
    pub order_number: String,
    pub items: Vec<ComponentIssueItem>,
}

// ============================================================================
// Component issue envelope builder
// ============================================================================

// ============================================================================
// FG receipt event payloads
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FgReceiptRequestedPayload {
    pub work_order_id: Uuid,
    pub tenant_id: String,
    pub order_number: String,
    pub item_id: Uuid,
    pub warehouse_id: Uuid,
    pub quantity: i64,
    pub currency: String,
}

// ============================================================================
// FG receipt envelope builder
// ============================================================================

pub fn build_fg_receipt_requested_envelope(
    work_order_id: Uuid,
    tenant_id: String,
    order_number: String,
    item_id: Uuid,
    warehouse_id: Uuid,
    quantity: i64,
    currency: String,
    correlation_id: String,
    causation_id: Option<String>,
) -> event_bus::EventEnvelope<FgReceiptRequestedPayload> {
    create_production_envelope(
        tenant_id.clone(),
        ProductionEventType::FgReceiptRequested.as_str().to_string(),
        correlation_id,
        causation_id,
        FgReceiptRequestedPayload {
            work_order_id,
            tenant_id,
            order_number,
            item_id,
            warehouse_id,
            quantity,
            currency,
        },
    )
}

pub fn build_component_issue_requested_envelope(
    work_order_id: Uuid,
    tenant_id: String,
    order_number: String,
    items: Vec<ComponentIssueItem>,
    correlation_id: String,
    causation_id: Option<String>,
) -> event_bus::EventEnvelope<ComponentIssueRequestedPayload> {
    create_production_envelope(
        tenant_id.clone(),
        ProductionEventType::ComponentIssueRequested
            .as_str()
            .to_string(),
        correlation_id,
        causation_id,
        ComponentIssueRequestedPayload {
            work_order_id,
            tenant_id,
            order_number,
            items,
        },
    )
}

// ============================================================================
// Time entry event payloads
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
pub struct TimeEntryCreatedPayload {
    pub time_entry_id: Uuid,
    pub work_order_id: Uuid,
    pub operation_id: Option<Uuid>,
    pub tenant_id: String,
    pub actor_id: String,
    pub start_ts: chrono::DateTime<chrono::Utc>,
    pub end_ts: Option<chrono::DateTime<chrono::Utc>>,
    pub minutes: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TimeEntryStoppedPayload {
    pub time_entry_id: Uuid,
    pub work_order_id: Uuid,
    pub operation_id: Option<Uuid>,
    pub tenant_id: String,
    pub actor_id: String,
    pub start_ts: chrono::DateTime<chrono::Utc>,
    pub end_ts: chrono::DateTime<chrono::Utc>,
    pub minutes: i32,
}

// ============================================================================
// Time entry envelope builders
// ============================================================================

pub fn build_time_entry_created_envelope(
    time_entry_id: Uuid,
    work_order_id: Uuid,
    operation_id: Option<Uuid>,
    tenant_id: String,
    actor_id: String,
    start_ts: chrono::DateTime<chrono::Utc>,
    end_ts: Option<chrono::DateTime<chrono::Utc>>,
    minutes: Option<i32>,
    correlation_id: String,
    causation_id: Option<String>,
) -> event_bus::EventEnvelope<TimeEntryCreatedPayload> {
    create_production_envelope(
        tenant_id.clone(),
        ProductionEventType::TimeEntryCreated.as_str().to_string(),
        correlation_id,
        causation_id,
        TimeEntryCreatedPayload {
            time_entry_id,
            work_order_id,
            operation_id,
            tenant_id,
            actor_id,
            start_ts,
            end_ts,
            minutes,
        },
    )
}

pub fn build_time_entry_stopped_envelope(
    time_entry_id: Uuid,
    work_order_id: Uuid,
    operation_id: Option<Uuid>,
    tenant_id: String,
    actor_id: String,
    start_ts: chrono::DateTime<chrono::Utc>,
    end_ts: chrono::DateTime<chrono::Utc>,
    minutes: i32,
    correlation_id: String,
    causation_id: Option<String>,
) -> event_bus::EventEnvelope<TimeEntryStoppedPayload> {
    create_production_envelope(
        tenant_id.clone(),
        ProductionEventType::TimeEntryStopped.as_str().to_string(),
        correlation_id,
        causation_id,
        TimeEntryStoppedPayload {
            time_entry_id,
            work_order_id,
            operation_id,
            tenant_id,
            actor_id,
            start_ts,
            end_ts,
            minutes,
        },
    )
}

// ============================================================================
// Time entry approved event payload
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
pub struct TimeEntryApprovedPayload {
    pub time_entry_id: Uuid,
    pub work_order_id: Uuid,
    pub operation_id: Option<Uuid>,
    pub tenant_id: String,
    pub actor_id: String,
    pub minutes: i32,
    pub approved_by: String,
    pub approved_at: chrono::DateTime<chrono::Utc>,
}

pub fn build_time_entry_approved_envelope(
    time_entry_id: Uuid,
    work_order_id: Uuid,
    operation_id: Option<Uuid>,
    tenant_id: String,
    actor_id: String,
    minutes: i32,
    approved_by: String,
    approved_at: chrono::DateTime<chrono::Utc>,
    correlation_id: String,
    causation_id: Option<String>,
) -> event_bus::EventEnvelope<TimeEntryApprovedPayload> {
    create_production_envelope(
        tenant_id.clone(),
        ProductionEventType::TimeEntryApproved.as_str().to_string(),
        correlation_id,
        causation_id,
        TimeEntryApprovedPayload {
            time_entry_id,
            work_order_id,
            operation_id,
            tenant_id,
            actor_id,
            minutes,
            approved_by,
            approved_at,
        },
    )
}

// ============================================================================
// Downtime event payloads
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
pub struct DowntimeStartedPayload {
    pub downtime_id: Uuid,
    pub tenant_id: String,
    pub workcenter_id: Uuid,
    pub reason: String,
    pub reason_code: Option<String>,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub started_by: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DowntimeEndedPayload {
    pub downtime_id: Uuid,
    pub tenant_id: String,
    pub workcenter_id: Uuid,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub ended_at: chrono::DateTime<chrono::Utc>,
    pub ended_by: Option<String>,
}

// ============================================================================
// Downtime envelope builders
// ============================================================================

pub fn build_downtime_started_envelope(
    downtime_id: Uuid,
    tenant_id: String,
    workcenter_id: Uuid,
    reason: String,
    reason_code: Option<String>,
    started_at: chrono::DateTime<chrono::Utc>,
    started_by: Option<String>,
    correlation_id: String,
    causation_id: Option<String>,
) -> event_bus::EventEnvelope<DowntimeStartedPayload> {
    create_production_envelope(
        tenant_id.clone(),
        ProductionEventType::DowntimeStarted.as_str().to_string(),
        correlation_id,
        causation_id,
        DowntimeStartedPayload {
            downtime_id,
            tenant_id,
            workcenter_id,
            reason,
            reason_code,
            started_at,
            started_by,
        },
    )
}

pub fn build_downtime_ended_envelope(
    downtime_id: Uuid,
    tenant_id: String,
    workcenter_id: Uuid,
    started_at: chrono::DateTime<chrono::Utc>,
    ended_at: chrono::DateTime<chrono::Utc>,
    ended_by: Option<String>,
    correlation_id: String,
    causation_id: Option<String>,
) -> event_bus::EventEnvelope<DowntimeEndedPayload> {
    create_production_envelope(
        tenant_id.clone(),
        ProductionEventType::DowntimeEnded.as_str().to_string(),
        correlation_id,
        causation_id,
        DowntimeEndedPayload {
            downtime_id,
            tenant_id,
            workcenter_id,
            started_at,
            ended_at,
            ended_by,
        },
    )
}
