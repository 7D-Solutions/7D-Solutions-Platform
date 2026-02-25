//! Shipment service — Guard→Mutation→Outbox for all shipment lifecycle operations.
//!
//! Invariants:
//! - Direction determines which state machine applies (inbound vs outbound)
//! - Every status transition is validated by state_machine + guards
//! - Every mutation writes its event to the outbox atomically in the same tx
//! - Inbound close: qty accounting enforced transactionally
//! - Outbound ship: qty + single-ship enforced transactionally
//! - All queries filter by tenant_id

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

use super::guards::{
    run_inbound_guards, run_outbound_guards, GuardError, InboundGuardContext,
    OutboundGuardContext,
};
use super::state_machine::{validate_inbound, validate_outbound, TransitionError};
use super::types::{Direction, InboundStatus, OutboundStatus};
use crate::db::repository::ShipmentRepository;
use crate::outbox;

// ── Domain model ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Shipment {
    pub id: Uuid,
    pub tenant_id: Uuid,
    #[sqlx(try_from = "String")]
    pub direction: Direction,
    pub status: String,
    pub carrier_party_id: Option<Uuid>,
    pub tracking_number: Option<String>,
    pub freight_cost_minor: Option<i64>,
    pub currency: Option<String>,
    pub expected_arrival_date: Option<DateTime<Utc>>,
    pub arrived_at: Option<DateTime<Utc>>,
    pub shipped_at: Option<DateTime<Utc>>,
    pub delivered_at: Option<DateTime<Utc>>,
    pub closed_at: Option<DateTime<Utc>>,
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ── Request types ─────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct TransitionRequest {
    pub status: String,
    pub arrived_at: Option<DateTime<Utc>>,
    pub shipped_at: Option<DateTime<Utc>>,
    pub delivered_at: Option<DateTime<Utc>>,
    pub closed_at: Option<DateTime<Utc>>,
}

// ── Errors ────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum ShipmentError {
    #[error("Shipment not found")]
    NotFound,

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Transition error: {0}")]
    Transition(#[from] TransitionError),

    #[error("Guard error: {0}")]
    Guard(#[from] GuardError),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ── Event subjects ────────────────────────────────────────────

pub mod subjects {
    pub const SHIPMENT_CREATED: &str = "shipping.shipment.created";
    pub const SHIPMENT_STATUS_CHANGED: &str = "shipping.shipment.status_changed";
    pub const INBOUND_CLOSED: &str = "shipping.inbound.closed";
    pub const OUTBOUND_SHIPPED: &str = "shipping.outbound.shipped";
    pub const OUTBOUND_DELIVERED: &str = "shipping.outbound.delivered";
}

// ── Service ──────────────────────────────────────────────────

pub struct ShipmentService;

impl ShipmentService {
    /// Transition a shipment's status with direction-specific state machine
    /// and guard enforcement. All invariants are checked within the same
    /// database transaction as the mutation and outbox write.
    pub async fn transition(
        pool: &PgPool,
        shipment_id: Uuid,
        tenant_id: Uuid,
        req: &TransitionRequest,
    ) -> Result<Shipment, ShipmentError> {
        let mut tx = pool.begin().await?;

        let current = ShipmentRepository::get_shipment_for_update(&mut tx, shipment_id, tenant_id)
            .await?
            .ok_or(ShipmentError::NotFound)?;

        let from_status = &current.status;

        let lines = ShipmentRepository::get_line_qtys_tx(&mut tx, shipment_id, tenant_id).await?;

        let event_type = match current.direction {
            Direction::Inbound => {
                let from = InboundStatus::from_str_value(from_status)
                    .map_err(|e| ShipmentError::Validation(e.to_string()))?;
                let to = InboundStatus::from_str_value(&req.status)
                    .map_err(|e| ShipmentError::Validation(e.to_string()))?;

                validate_inbound(from, to)?;

                let ctx = InboundGuardContext {
                    arrived_at: req.arrived_at,
                    closed_at: req.closed_at,
                    lines,
                    already_shipped_at: current.shipped_at,
                };
                run_inbound_guards(to, &ctx)?;

                match to {
                    InboundStatus::Closed => subjects::INBOUND_CLOSED,
                    _ => subjects::SHIPMENT_STATUS_CHANGED,
                }
            }
            Direction::Outbound => {
                let from = OutboundStatus::from_str_value(from_status)
                    .map_err(|e| ShipmentError::Validation(e.to_string()))?;
                let to = OutboundStatus::from_str_value(&req.status)
                    .map_err(|e| ShipmentError::Validation(e.to_string()))?;

                validate_outbound(from, to)?;

                let ctx = OutboundGuardContext {
                    shipped_at: req.shipped_at,
                    delivered_at: req.delivered_at,
                    closed_at: req.closed_at,
                    lines,
                    already_shipped_at: current.shipped_at,
                };
                run_outbound_guards(to, &ctx)?;

                match to {
                    OutboundStatus::Shipped => subjects::OUTBOUND_SHIPPED,
                    OutboundStatus::Delivered => subjects::OUTBOUND_DELIVERED,
                    _ => subjects::SHIPMENT_STATUS_CHANGED,
                }
            }
        };

        // ── Mutation via repository ──
        let shipment = ShipmentRepository::update_shipment_status(
            &mut tx,
            shipment_id,
            tenant_id,
            &req.status,
            req.arrived_at,
            req.shipped_at,
            req.delivered_at,
            req.closed_at,
        )
        .await?;

        // ── Outbox ──
        let event_payload = serde_json::json!({
            "shipment_id": shipment_id,
            "tenant_id": tenant_id,
            "direction": current.direction.as_str(),
            "from_status": from_status,
            "to_status": &req.status,
        });
        let event_id = Uuid::new_v4();
        outbox::enqueue_event_tx(
            &mut tx,
            event_id,
            event_type,
            "shipment",
            &shipment_id.to_string(),
            &tenant_id.to_string(),
            &event_payload,
        )
        .await?;

        tx.commit().await?;
        Ok(shipment)
    }

    /// Find a shipment by ID within a tenant.
    pub async fn find_by_id(
        pool: &PgPool,
        id: Uuid,
        tenant_id: Uuid,
    ) -> Result<Option<Shipment>, ShipmentError> {
        ShipmentRepository::get_shipment(pool, id, tenant_id)
            .await
            .map_err(ShipmentError::Database)
    }
}
