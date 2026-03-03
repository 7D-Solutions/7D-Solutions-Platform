//! Shipping document request service — Guard → Mutation → Outbox.
//!
//! Invariants:
//! - Every status transition is validated by the state machine
//! - Every mutation writes its event to the outbox atomically in the same tx
//! - Idempotent creation via idempotency_key (unique per tenant)
//! - All queries filter by tenant_id

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

use super::state_machine::{validate_doc_status, DocStatusTransitionError};
use super::types::{DocRequestStatus, DocType};
use crate::db::shipping_doc_repo::ShippingDocRepo;
use crate::outbox;

// ── Domain model ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ShippingDocRequest {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub shipment_id: Uuid,
    pub doc_type: String,
    pub status: String,
    pub payload_ref: Option<String>,
    pub idempotency_key: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ── Request types ─────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateDocRequest {
    pub shipment_id: Uuid,
    pub doc_type: String,
    pub payload_ref: Option<String>,
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TransitionStatusRequest {
    pub status: String,
}

// ── Errors ────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum ShippingDocError {
    #[error("Shipping doc request not found")]
    NotFound,

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Transition error: {0}")]
    Transition(#[from] DocStatusTransitionError),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ── Event type constants ─────────────────────────────────────

pub mod subjects {
    pub const DOC_REQUESTED: &str = "sr.shipping_doc.requested";
    pub const DOC_STATUS_CHANGED: &str = "sr.shipping_doc.status_changed";
}

// ── Service ──────────────────────────────────────────────────

pub struct ShippingDocService;

impl ShippingDocService {
    /// Create a shipping document request: Guard → Mutation → Outbox.
    ///
    /// Idempotent: if `idempotency_key` matches an existing request within the
    /// same tenant, returns the existing request without creating a duplicate.
    pub async fn create(
        pool: &PgPool,
        tenant_id: Uuid,
        req: &CreateDocRequest,
    ) -> Result<ShippingDocRequest, ShippingDocError> {
        // ── Guard: validate doc_type ──
        DocType::from_str_value(&req.doc_type)
            .map_err(|e| ShippingDocError::Validation(e.to_string()))?;

        let mut tx = pool.begin().await?;

        // ── Idempotency check ──
        if let Some(ref key) = req.idempotency_key {
            if let Some(existing) =
                ShippingDocRepo::find_by_idempotency_key_tx(&mut tx, tenant_id, key).await?
            {
                tx.commit().await?;
                return Ok(existing);
            }
        }

        // ── Mutation: insert request ──
        let doc_req = ShippingDocRepo::insert_tx(
            &mut tx,
            tenant_id,
            req.shipment_id,
            &req.doc_type,
            req.payload_ref.as_deref(),
            req.idempotency_key.as_deref(),
        )
        .await?;

        // ── Outbox ──
        let event_payload = serde_json::json!({
            "doc_request_id": doc_req.id,
            "tenant_id": tenant_id,
            "shipment_id": req.shipment_id,
            "doc_type": &req.doc_type,
            "status": "requested",
        });

        let event_id = Uuid::new_v4();
        outbox::enqueue_event_tx(
            &mut tx,
            event_id,
            subjects::DOC_REQUESTED,
            "shipping_doc_request",
            &doc_req.id.to_string(),
            &tenant_id.to_string(),
            &event_payload,
        )
        .await?;

        tx.commit().await?;
        Ok(doc_req)
    }

    /// Transition a doc request's status.
    ///
    /// Guard → Mutation → Outbox, all within a single transaction.
    pub async fn transition_status(
        pool: &PgPool,
        doc_request_id: Uuid,
        tenant_id: Uuid,
        req: &TransitionStatusRequest,
    ) -> Result<ShippingDocRequest, ShippingDocError> {
        let to = DocRequestStatus::from_str_value(&req.status)
            .map_err(|e| ShippingDocError::Validation(e.to_string()))?;

        let mut tx = pool.begin().await?;

        // ── Guard: load current state with row lock ──
        let current = ShippingDocRepo::get_for_update_tx(&mut tx, doc_request_id, tenant_id)
            .await?
            .ok_or(ShippingDocError::NotFound)?;

        let from = DocRequestStatus::from_str_value(&current.status)
            .map_err(|e| ShippingDocError::Validation(e.to_string()))?;

        // ── Guard: validate state machine transition ──
        validate_doc_status(from, to)?;

        // ── Mutation ──
        let updated =
            ShippingDocRepo::update_status_tx(&mut tx, doc_request_id, tenant_id, to.as_str())
                .await?;

        // ── Outbox ──
        let event_payload = serde_json::json!({
            "doc_request_id": doc_request_id,
            "tenant_id": tenant_id,
            "shipment_id": current.shipment_id,
            "doc_type": current.doc_type,
            "from_status": from.as_str(),
            "to_status": to.as_str(),
        });

        let event_id = Uuid::new_v4();
        outbox::enqueue_event_tx(
            &mut tx,
            event_id,
            subjects::DOC_STATUS_CHANGED,
            "shipping_doc_request",
            &doc_request_id.to_string(),
            &tenant_id.to_string(),
            &event_payload,
        )
        .await?;

        tx.commit().await?;
        Ok(updated)
    }

    /// Find a doc request by ID within a tenant.
    pub async fn find_by_id(
        pool: &PgPool,
        id: Uuid,
        tenant_id: Uuid,
    ) -> Result<Option<ShippingDocRequest>, ShippingDocError> {
        ShippingDocRepo::get(pool, id, tenant_id)
            .await
            .map_err(ShippingDocError::Database)
    }

    /// List doc requests for a shipment within a tenant.
    pub async fn list_by_shipment(
        pool: &PgPool,
        shipment_id: Uuid,
        tenant_id: Uuid,
    ) -> Result<Vec<ShippingDocRequest>, ShippingDocError> {
        ShippingDocRepo::list_by_shipment(pool, shipment_id, tenant_id)
            .await
            .map_err(ShippingDocError::Database)
    }
}
