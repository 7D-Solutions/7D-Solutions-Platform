//! Composite outbound shipment flow.
//!
//! Orchestrates the full state machine for shipping an outbound order:
//! 1. Validate shipment exists and is in packed state
//! 2. Collect source work order IDs from lines
//! 3. Quality gate: check QI service for "held" inspections on those WOs
//! 4. If holds and no override_reason → block (403)
//! 5. If holds and override_reason present → require quality_inspection.mutate permission
//! 6. Transition: packed → shipped (inventory issue + outbox event atomically)
//!
//! Invariant: all existing individual endpoints remain unchanged. This is additive.
//! Security boundary: quality gate bypass MUST check caller permissions — never skip.

use chrono::Utc;
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

use crate::db::repository::ShipmentRepository;
use crate::domain::shipments::service::ShipmentService;
use crate::domain::shipments::types::{Direction, OutboundStatus};
use crate::domain::shipments::{Shipment, ShipmentError, TransitionRequest};
use crate::integrations::inventory_client::InventoryIntegration;
use crate::integrations::quality_gate_client::{QualityGateError, QualityGateIntegration};

// ── Errors ────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum OutboundShipError {
    #[error("Shipment not found")]
    NotFound,

    #[error("Shipment is not outbound")]
    NotOutbound,

    #[error("Shipment must be in packed state to ship outbound (current: {current})")]
    NotPackable { current: String },

    #[error("Quality gate: {hold_count} final inspection(s) on hold — supply override_reason to bypass")]
    QualityGateHold { hold_count: usize },

    #[error("Quality gate bypass requires quality_inspection.mutate permission")]
    InsufficientPermissions,

    #[error(transparent)]
    Shipment(#[from] ShipmentError),

    #[error("Quality inspection service error: {0}")]
    QiIntegration(String),
}

impl From<QualityGateError> for OutboundShipError {
    fn from(e: QualityGateError) -> Self {
        OutboundShipError::QiIntegration(e.to_string())
    }
}

// ── Request ───────────────────────────────────────────────────

/// Input for the composite outbound ship flow.
pub struct OutboundShipRequest {
    pub shipment_id: Uuid,
    pub tenant_id: Uuid,
    /// Timestamp to record as shipped_at. Defaults to now.
    pub shipped_at: Option<chrono::DateTime<Utc>>,
    /// If supplied, the caller intends to override a quality gate hold.
    /// Requires `quality_inspection.mutate` permission.
    pub override_reason: Option<String>,
    /// Whether the caller holds `quality_inspection.mutate` permission.
    /// Populated from VerifiedClaims.perms in the handler.
    pub caller_can_override_qi: bool,
}

// ── Service ───────────────────────────────────────────────────

pub struct OutboundShipService;

impl OutboundShipService {
    /// Execute the composite outbound ship flow.
    ///
    /// Returns the updated shipment in `shipped` status on success.
    pub async fn execute(
        pool: &PgPool,
        req: OutboundShipRequest,
        inventory: &InventoryIntegration,
        quality_gate: &QualityGateIntegration,
    ) -> Result<Shipment, OutboundShipError> {
        // ── Step 1: Validate shipment exists, is outbound, is packed ──
        let shipment = ShipmentService::find_by_id(pool, req.shipment_id, req.tenant_id)
            .await?
            .ok_or(OutboundShipError::NotFound)?;

        if shipment.direction != Direction::Outbound {
            return Err(OutboundShipError::NotOutbound);
        }

        let status = OutboundStatus::from_str_value(&shipment.status)
            .map_err(|e| ShipmentError::Validation(e.to_string()))?;

        if status != OutboundStatus::Packed {
            return Err(OutboundShipError::NotPackable {
                current: shipment.status.clone(),
            });
        }

        // ── Step 2: Collect source WO IDs from shipment lines ──
        let lines = ShipmentRepository::get_lines_for_shipment(pool, req.shipment_id, req.tenant_id)
            .await
            .map_err(|e| ShipmentError::Database(e))?;

        let wo_ids: Vec<Uuid> = lines
            .iter()
            .filter(|l| l.source_ref_type.as_deref() == Some("work_order"))
            .filter_map(|l| l.source_ref_id)
            .collect();

        // ── Step 3: Quality gate check ──
        let holds = quality_gate.check_wo_holds(req.tenant_id, &wo_ids).await?;

        if !holds.is_empty() {
            match req.override_reason {
                None => {
                    return Err(OutboundShipError::QualityGateHold {
                        hold_count: holds.len(),
                    });
                }
                Some(ref reason) => {
                    if !req.caller_can_override_qi {
                        // override_reason was provided but caller lacks permission
                        return Err(OutboundShipError::InsufficientPermissions);
                    }
                    tracing::warn!(
                        shipment_id = %req.shipment_id,
                        tenant_id = %req.tenant_id,
                        hold_count = holds.len(),
                        override_reason = %reason,
                        "Quality gate bypassed by authorized caller"
                    );
                }
            }
        }

        // ── Step 4: Transition packed → shipped (inventory issue + outbox) ──
        let transition = TransitionRequest {
            status: "shipped".to_string(),
            arrived_at: None,
            shipped_at: Some(req.shipped_at.unwrap_or_else(Utc::now)),
            delivered_at: None,
            closed_at: None,
        };

        ShipmentService::transition(pool, req.shipment_id, req.tenant_id, &transition, inventory)
            .await
            .map_err(OutboundShipError::Shipment)
    }
}
