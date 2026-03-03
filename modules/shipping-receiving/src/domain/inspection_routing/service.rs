//! Inspection routing service — Guard → Mutation → Outbox.
//!
//! Invariants:
//! - Only inbound shipments can be routed
//! - Shipment must be in "receiving" status
//! - Each line can only be routed once
//! - Idempotent: same idempotency_key returns existing routing without re-routing
//! - Routing decision and outbox event are committed atomically

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

use super::types::RouteDecision;
use crate::db::inspection_routing_repo::{InspectionRoutingRepo, InspectionRoutingRow};
use crate::db::repository::ShipmentRepository;
use crate::domain::shipments::types::{Direction, InboundStatus};
use crate::outbox;

#[derive(Debug, Error)]
pub enum RoutingError {
    #[error("Shipment not found")]
    ShipmentNotFound,

    #[error("Shipment line not found")]
    LineNotFound,

    #[error("Routing is only valid for inbound shipments")]
    NotInbound,

    #[error("Shipment must be in receiving status to route (current: {current})")]
    NotReceiving { current: String },

    #[error("Line {line_id} is already routed as '{decision}'")]
    AlreadyRouted {
        line_id: Uuid,
        decision: String,
    },

    #[error("Invalid route decision: {0}")]
    InvalidDecision(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

/// Request to route a shipment line.
#[derive(Debug, serde::Deserialize)]
pub struct RouteLineRequest {
    pub route_decision: String,
    pub reason: Option<String>,
    pub idempotency_key: Option<String>,
}

pub struct InspectionRoutingService;

impl InspectionRoutingService {
    /// Route a shipment line to stock or inspection.
    ///
    /// Guard → Mutation → Outbox, all within a single transaction.
    /// Idempotent: if `idempotency_key` matches an existing routing, returns it.
    pub async fn route_line(
        pool: &PgPool,
        shipment_id: Uuid,
        line_id: Uuid,
        tenant_id: Uuid,
        routed_by: Option<Uuid>,
        req: &RouteLineRequest,
    ) -> Result<InspectionRoutingRow, RoutingError> {
        let decision = RouteDecision::from_str_value(&req.route_decision)
            .map_err(|e| RoutingError::InvalidDecision(e.to_string()))?;

        let mut tx = pool.begin().await?;

        // ── Idempotency check ──
        if let Some(ref key) = req.idempotency_key {
            if let Some(existing) =
                InspectionRoutingRepo::find_by_idempotency_key_tx(&mut tx, tenant_id, key).await?
            {
                tx.commit().await?;
                return Ok(existing);
            }
        }

        // ── Guard: shipment exists, is inbound, is in receiving status ──
        let shipment = ShipmentRepository::get_shipment_for_update(&mut tx, shipment_id, tenant_id)
            .await?
            .ok_or(RoutingError::ShipmentNotFound)?;

        if shipment.direction != Direction::Inbound {
            return Err(RoutingError::NotInbound);
        }

        let status = InboundStatus::from_str_value(&shipment.status)
            .map_err(|e| RoutingError::InvalidDecision(e.to_string()))?;
        if status != InboundStatus::Receiving {
            return Err(RoutingError::NotReceiving {
                current: shipment.status.clone(),
            });
        }

        // ── Guard: line exists for this shipment + tenant ──
        let line_exists: Option<(Uuid,)> = sqlx::query_as(
            "SELECT id FROM shipment_lines WHERE id = $1 AND shipment_id = $2 AND tenant_id = $3",
        )
        .bind(line_id)
        .bind(shipment_id)
        .bind(tenant_id)
        .fetch_optional(&mut *tx)
        .await?;

        if line_exists.is_none() {
            return Err(RoutingError::LineNotFound);
        }

        // ── Guard: line not already routed ──
        if let Some(existing) =
            InspectionRoutingRepo::find_by_line_tx(&mut tx, tenant_id, line_id).await?
        {
            return Err(RoutingError::AlreadyRouted {
                line_id,
                decision: existing.route_decision,
            });
        }

        // ── Mutation ──
        let now: DateTime<Utc> = Utc::now();
        let routing = InspectionRoutingRepo::insert_routing_tx(
            &mut tx,
            tenant_id,
            shipment_id,
            line_id,
            decision.as_str(),
            req.reason.as_deref(),
            routed_by,
            now,
            req.idempotency_key.as_deref(),
        )
        .await?;

        // ── Outbox ──
        let event_payload = serde_json::json!({
            "routing_id": routing.id,
            "tenant_id": tenant_id,
            "shipment_id": shipment_id,
            "shipment_line_id": line_id,
            "route_decision": decision.as_str(),
            "reason": req.reason,
            "routed_by": routed_by,
            "routed_at": now,
        });

        let event_id = Uuid::new_v4();
        outbox::enqueue_event_tx(
            &mut tx,
            event_id,
            decision.event_type(),
            "inspection_routing",
            &routing.id.to_string(),
            &tenant_id.to_string(),
            &event_payload,
        )
        .await?;

        tx.commit().await?;
        Ok(routing)
    }

    /// List all inspection routings for a shipment.
    pub async fn list_for_shipment(
        pool: &PgPool,
        shipment_id: Uuid,
        tenant_id: Uuid,
    ) -> Result<Vec<InspectionRoutingRow>, RoutingError> {
        InspectionRoutingRepo::get_routings_for_shipment(pool, tenant_id, shipment_id)
            .await
            .map_err(RoutingError::Database)
    }
}
