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

use std::collections::HashMap;

use super::guards::{
    run_inbound_guards, run_outbound_guards, GuardError, InboundGuardContext, OutboundGuardContext,
};
use super::state_machine::{validate_inbound, validate_outbound, TransitionError};
use super::types::{Direction, InboundStatus, OutboundStatus};
use crate::db::repository::ShipmentRepository;
use crate::events::{
    InboundClosedLine, InboundClosedPayload, OutboundDeliveredPayload, OutboundShippedLine,
    OutboundShippedPayload, ShipmentStatusChangedPayload, EVENT_TYPE_INBOUND_CLOSED,
    EVENT_TYPE_OUTBOUND_DELIVERED, EVENT_TYPE_OUTBOUND_SHIPPED, EVENT_TYPE_SHIPMENT_STATUS_CHANGED,
};
use crate::integrations::inventory_client::InventoryIntegration;
use crate::outbox;

// ── Domain model ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, utoipa::ToSchema)]
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

    #[error("Inventory integration error: {0}")]
    InventoryIntegration(String),
}

// ── Event type constants (canonical, from contracts) ─────────
// Re-exported here for backward compatibility with internal references.

// ── Service ──────────────────────────────────────────────────

pub struct ShipmentService;

impl ShipmentService {
    /// Transition a shipment's status with direction-specific state machine
    /// and guard enforcement. All invariants are checked within the same
    /// database transaction as the mutation and outbox write.
    ///
    /// Inventory integration: on inbound close, creates a receipt per accepted
    /// line; on outbound ship, creates an issue per shipped line. If the
    /// inventory call fails, the entire transaction is rolled back.
    pub async fn transition(
        pool: &PgPool,
        shipment_id: Uuid,
        tenant_id: Uuid,
        req: &TransitionRequest,
        inventory: &InventoryIntegration,
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
                    lines: lines.clone(),
                    already_shipped_at: current.shipped_at,
                };
                run_inbound_guards(to, &ctx)?;

                match to {
                    InboundStatus::Closed => EVENT_TYPE_INBOUND_CLOSED,
                    _ => EVENT_TYPE_SHIPMENT_STATUS_CHANGED,
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
                    lines: lines.clone(),
                    already_shipped_at: current.shipped_at,
                };
                run_outbound_guards(to, &ctx)?;

                match to {
                    OutboundStatus::Shipped => EVENT_TYPE_OUTBOUND_SHIPPED,
                    OutboundStatus::Delivered => EVENT_TYPE_OUTBOUND_DELIVERED,
                    _ => EVENT_TYPE_SHIPMENT_STATUS_CHANGED,
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

        // ── Inventory integration ──
        let needs_inventory =
            event_type == EVENT_TYPE_INBOUND_CLOSED || event_type == EVENT_TYPE_OUTBOUND_SHIPPED;
        let inventory_refs = if needs_inventory {
            Self::process_inventory(
                &mut tx,
                inventory,
                &current.direction,
                shipment_id,
                tenant_id,
                current.currency.as_deref().unwrap_or("usd"),
            )
            .await?
        } else {
            Vec::new()
        };

        // ── Build typed event payload ──
        let ref_map: HashMap<Uuid, Uuid> = inventory_refs.into_iter().collect();
        let now = Utc::now();
        let tenant_str = tenant_id.to_string();

        let event_id = Uuid::new_v4();
        match event_type {
            EVENT_TYPE_OUTBOUND_SHIPPED => {
                let payload = OutboundShippedPayload {
                    tenant_id: tenant_str.clone(),
                    shipment_id,
                    lines: lines
                        .iter()
                        .filter(|l| l.qty_shipped > 0)
                        .map(|l| OutboundShippedLine {
                            line_id: l.line_id,
                            sku: l.sku.clone(),
                            qty_shipped: l.qty_shipped,
                            issue_id: ref_map.get(&l.line_id).copied(),
                            source_ref_type: l.source_ref_type.clone(),
                            source_ref_id: l.source_ref_id,
                        })
                        .collect(),
                    shipped_at: req.shipped_at.unwrap_or(now),
                    tracking_number: current.tracking_number.clone(),
                    carrier_party_id: current.carrier_party_id,
                };
                outbox::enqueue_event_tx(
                    &mut tx,
                    event_id,
                    event_type,
                    "shipment",
                    &shipment_id.to_string(),
                    &tenant_str,
                    &payload,
                )
                .await?;
            }
            EVENT_TYPE_INBOUND_CLOSED => {
                let payload = InboundClosedPayload {
                    tenant_id: tenant_str.clone(),
                    shipment_id,
                    lines: lines
                        .iter()
                        .filter(|l| l.qty_accepted > 0 || l.qty_rejected > 0)
                        .map(|l| InboundClosedLine {
                            line_id: l.line_id,
                            sku: l.sku.clone(),
                            qty_accepted: l.qty_accepted,
                            qty_rejected: l.qty_rejected,
                            receipt_id: ref_map.get(&l.line_id).copied(),
                        })
                        .collect(),
                    closed_at: req.closed_at.unwrap_or(now),
                };
                outbox::enqueue_event_tx(
                    &mut tx,
                    event_id,
                    event_type,
                    "shipment",
                    &shipment_id.to_string(),
                    &tenant_str,
                    &payload,
                )
                .await?;
            }
            EVENT_TYPE_OUTBOUND_DELIVERED => {
                let payload = OutboundDeliveredPayload {
                    tenant_id: tenant_str.clone(),
                    shipment_id,
                    delivered_at: req.delivered_at.unwrap_or(now),
                };
                outbox::enqueue_event_tx(
                    &mut tx,
                    event_id,
                    event_type,
                    "shipment",
                    &shipment_id.to_string(),
                    &tenant_str,
                    &payload,
                )
                .await?;
            }
            _ => {
                // STATUS_CHANGED (generic transition)
                let payload = ShipmentStatusChangedPayload {
                    tenant_id: tenant_str.clone(),
                    shipment_id,
                    direction: current.direction,
                    old_status: from_status.to_string(),
                    new_status: req.status.clone(),
                    changed_by: "system".to_string(),
                    changed_at: now,
                };
                outbox::enqueue_event_tx(
                    &mut tx,
                    event_id,
                    event_type,
                    "shipment",
                    &shipment_id.to_string(),
                    &tenant_str,
                    &payload,
                )
                .await?;
            }
        }

        tx.commit().await?;
        Ok(shipment)
    }

    /// Process inventory movements for a shipment's lines.
    ///
    /// - Inbound close: create a receipt for each line with qty_accepted > 0
    /// - Outbound ship: create an issue for each line with qty_shipped > 0
    ///
    /// Lines where `inventory_ref_id` is already set are skipped (exactly-once).
    /// Returns (line_id, inventory_ref_id) pairs for the outbox event payload.
    async fn process_inventory(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        inventory: &InventoryIntegration,
        direction: &Direction,
        shipment_id: Uuid,
        tenant_id: Uuid,
        currency: &str,
    ) -> Result<Vec<(Uuid, Uuid)>, ShipmentError> {
        let lines = ShipmentRepository::get_inventory_lines_tx(tx, shipment_id, tenant_id).await?;

        let mut refs = Vec::new();

        for line in &lines {
            // Skip lines that already have an inventory reference (idempotency guard)
            if line.inventory_ref_id.is_some() {
                continue;
            }

            let qty = match direction {
                Direction::Inbound => line.qty_accepted,
                Direction::Outbound => line.qty_shipped,
            };

            if qty <= 0 {
                continue;
            }

            let warehouse_id = match line.warehouse_id {
                Some(wh) => wh,
                None => continue, // no warehouse → not inventory-tracked, skip
            };

            let ref_id = match direction {
                Direction::Inbound => inventory
                    .create_receipt(tenant_id, shipment_id, line.id, warehouse_id, qty, currency)
                    .await
                    .map_err(|e| ShipmentError::InventoryIntegration(e.to_string()))?,
                Direction::Outbound => inventory
                    .create_issue(tenant_id, shipment_id, line.id, warehouse_id, qty, currency)
                    .await
                    .map_err(|e| ShipmentError::InventoryIntegration(e.to_string()))?,
            };

            ShipmentRepository::set_inventory_ref_id_tx(tx, line.id, tenant_id, ref_id).await?;
            refs.push((line.id, ref_id));
        }

        Ok(refs)
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
