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
        }
    }
}
