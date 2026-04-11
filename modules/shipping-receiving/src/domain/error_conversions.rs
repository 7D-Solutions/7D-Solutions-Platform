use platform_http_contracts::ApiError;

use super::inspection_routing::RoutingError;
use super::outbound_ship::OutboundShipError;
use super::shipments::ShipmentError;

impl From<ShipmentError> for ApiError {
    fn from(err: ShipmentError) -> Self {
        match err {
            ShipmentError::NotFound => ApiError::not_found("Shipment not found"),
            ShipmentError::Validation(msg) => ApiError::new(422, "validation_error", msg),
            ShipmentError::Transition(t) => {
                ApiError::new(400, "invalid_transition", t.to_string())
            }
            ShipmentError::Guard(g) => ApiError::new(400, "guard_failed", g.to_string()),
            ShipmentError::Database(e) => {
                tracing::error!(error = %e, "database error");
                ApiError::internal("Internal server error")
            }
            ShipmentError::InventoryIntegration(msg) => {
                tracing::error!(error = %msg, "inventory integration error");
                ApiError::new(502, "inventory_error", "Inventory integration failed")
            }
        }
    }
}

impl From<OutboundShipError> for ApiError {
    fn from(err: OutboundShipError) -> Self {
        match err {
            OutboundShipError::NotFound => ApiError::not_found("Shipment not found"),
            OutboundShipError::NotOutbound => {
                ApiError::new(400, "validation_error", "Shipment is not outbound")
            }
            OutboundShipError::NotPackable { ref current } => ApiError::new(
                400,
                "invalid_state",
                format!(
                    "Shipment must be in packed state to ship outbound (current: {current})"
                ),
            ),
            OutboundShipError::QualityGateHold { hold_count } => ApiError::new(
                403,
                "quality_gate_hold",
                format!(
                    "{hold_count} final inspection(s) on hold — supply override_reason to bypass"
                ),
            ),
            OutboundShipError::InsufficientPermissions => ApiError::new(
                403,
                "insufficient_permissions",
                "Quality gate bypass requires quality_inspection.mutate permission",
            ),
            OutboundShipError::Shipment(e) => ApiError::from(e),
            OutboundShipError::QiIntegration(msg) => {
                tracing::error!(error = %msg, "quality inspection integration error");
                ApiError::new(502, "qi_integration_error", "Quality inspection service error")
            }
        }
    }
}

impl From<RoutingError> for ApiError {
    fn from(err: RoutingError) -> Self {
        match err {
            RoutingError::ShipmentNotFound => ApiError::not_found("Shipment not found"),
            RoutingError::LineNotFound => ApiError::not_found("Shipment line not found"),
            RoutingError::NotInbound => ApiError::new(
                400,
                "validation_error",
                "Routing is only valid for inbound shipments",
            ),
            RoutingError::NotReceiving { ref current } => ApiError::new(
                400,
                "validation_error",
                format!(
                    "Shipment must be in receiving status to route (current: {current})"
                ),
            ),
            RoutingError::AlreadyRouted { decision, .. } => {
                ApiError::conflict(format!("Line is already routed as '{decision}'"))
            }
            RoutingError::InvalidDecision(ref msg) => {
                ApiError::new(400, "validation_error", msg.clone())
            }
            RoutingError::Database(ref e) => {
                tracing::error!(error = %e, "database error in routing");
                ApiError::internal("Internal server error")
            }
        }
    }
}
