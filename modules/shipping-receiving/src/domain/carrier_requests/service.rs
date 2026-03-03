//! Carrier request service — Guard → Mutation → Outbox.
//!
//! Invariants:
//! - Every status transition is validated by the state machine
//! - Every mutation writes its event to the outbox atomically in the same tx
//! - Idempotent creation via idempotency_key (unique per tenant)
//! - All queries filter by tenant_id
//! - Every request/response is durably logged before acting on it

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

use super::state_machine::{validate_carrier_status, CarrierStatusTransitionError};
use super::types::{CarrierRequestStatus, CarrierRequestType};
use crate::db::carrier_request_repo::CarrierRequestRepo;
use crate::outbox;

// ── Domain model ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct CarrierRequest {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub shipment_id: Uuid,
    pub request_type: String,
    pub carrier_code: String,
    pub status: String,
    pub payload: serde_json::Value,
    pub response: Option<serde_json::Value>,
    pub idempotency_key: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ── Request types ─────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateCarrierRequest {
    pub shipment_id: Uuid,
    pub request_type: String,
    pub carrier_code: String,
    pub payload: serde_json::Value,
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TransitionCarrierRequest {
    pub status: String,
    pub response: Option<serde_json::Value>,
}

// ── Errors ────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum CarrierRequestError {
    #[error("Carrier request not found")]
    NotFound,

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Transition error: {0}")]
    Transition(#[from] CarrierStatusTransitionError),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ── Event type constants ─────────────────────────────────────

pub mod subjects {
    pub const CARRIER_REQUEST_CREATED: &str = "sr.carrier_request.created";
    pub const CARRIER_REQUEST_STATUS_CHANGED: &str = "sr.carrier_request.status_changed";
}

// ── Service ──────────────────────────────────────────────────

pub struct CarrierRequestService;

impl CarrierRequestService {
    /// Create a carrier integration request: Guard → Mutation → Outbox.
    ///
    /// Idempotent: if `idempotency_key` matches an existing request within the
    /// same tenant, returns the existing request without creating a duplicate.
    pub async fn create(
        pool: &PgPool,
        tenant_id: Uuid,
        req: &CreateCarrierRequest,
    ) -> Result<CarrierRequest, CarrierRequestError> {
        // ── Guard: validate request_type ──
        CarrierRequestType::from_str_value(&req.request_type)
            .map_err(|e| CarrierRequestError::Validation(e.to_string()))?;

        if req.carrier_code.is_empty() {
            return Err(CarrierRequestError::Validation(
                "carrier_code must not be empty".to_string(),
            ));
        }

        let mut tx = pool.begin().await?;

        // ── Idempotency check ──
        if let Some(ref key) = req.idempotency_key {
            if let Some(existing) =
                CarrierRequestRepo::find_by_idempotency_key_tx(&mut tx, tenant_id, key).await?
            {
                tx.commit().await?;
                return Ok(existing);
            }
        }

        // ── Mutation: insert request ──
        let carrier_req = CarrierRequestRepo::insert_tx(
            &mut tx,
            tenant_id,
            req.shipment_id,
            &req.request_type,
            &req.carrier_code,
            &req.payload,
            req.idempotency_key.as_deref(),
        )
        .await?;

        // ── Outbox ──
        let event_payload = serde_json::json!({
            "carrier_request_id": carrier_req.id,
            "tenant_id": tenant_id,
            "shipment_id": req.shipment_id,
            "request_type": &req.request_type,
            "carrier_code": &req.carrier_code,
            "status": "pending",
        });

        let event_id = Uuid::new_v4();
        outbox::enqueue_event_tx(
            &mut tx,
            event_id,
            subjects::CARRIER_REQUEST_CREATED,
            "carrier_request",
            &carrier_req.id.to_string(),
            &tenant_id.to_string(),
            &event_payload,
        )
        .await?;

        tx.commit().await?;
        Ok(carrier_req)
    }

    /// Transition a carrier request's status.
    ///
    /// Guard → Mutation → Outbox, all within a single transaction.
    /// When transitioning to `completed`, the response payload is stored.
    pub async fn transition_status(
        pool: &PgPool,
        carrier_request_id: Uuid,
        tenant_id: Uuid,
        req: &TransitionCarrierRequest,
    ) -> Result<CarrierRequest, CarrierRequestError> {
        let to = CarrierRequestStatus::from_str_value(&req.status)
            .map_err(|e| CarrierRequestError::Validation(e.to_string()))?;

        let mut tx = pool.begin().await?;

        // ── Guard: load current state with row lock ──
        let current =
            CarrierRequestRepo::get_for_update_tx(&mut tx, carrier_request_id, tenant_id)
                .await?
                .ok_or(CarrierRequestError::NotFound)?;

        let from = CarrierRequestStatus::from_str_value(&current.status)
            .map_err(|e| CarrierRequestError::Validation(e.to_string()))?;

        // ── Guard: validate state machine transition ──
        validate_carrier_status(from, to)?;

        // ── Mutation ──
        let updated = CarrierRequestRepo::update_status_tx(
            &mut tx,
            carrier_request_id,
            tenant_id,
            to.as_str(),
            req.response.as_ref(),
        )
        .await?;

        // ── Outbox ──
        let event_payload = serde_json::json!({
            "carrier_request_id": carrier_request_id,
            "tenant_id": tenant_id,
            "shipment_id": current.shipment_id,
            "request_type": current.request_type,
            "carrier_code": current.carrier_code,
            "from_status": from.as_str(),
            "to_status": to.as_str(),
        });

        let event_id = Uuid::new_v4();
        outbox::enqueue_event_tx(
            &mut tx,
            event_id,
            subjects::CARRIER_REQUEST_STATUS_CHANGED,
            "carrier_request",
            &carrier_request_id.to_string(),
            &tenant_id.to_string(),
            &event_payload,
        )
        .await?;

        tx.commit().await?;
        Ok(updated)
    }

    /// Find a carrier request by ID within a tenant.
    pub async fn find_by_id(
        pool: &PgPool,
        id: Uuid,
        tenant_id: Uuid,
    ) -> Result<Option<CarrierRequest>, CarrierRequestError> {
        CarrierRequestRepo::get(pool, id, tenant_id)
            .await
            .map_err(CarrierRequestError::Database)
    }

    /// List carrier requests for a shipment within a tenant.
    pub async fn list_by_shipment(
        pool: &PgPool,
        shipment_id: Uuid,
        tenant_id: Uuid,
    ) -> Result<Vec<CarrierRequest>, CarrierRequestError> {
        CarrierRequestRepo::list_by_shipment(pool, shipment_id, tenant_id)
            .await
            .map_err(CarrierRequestError::Database)
    }
}
