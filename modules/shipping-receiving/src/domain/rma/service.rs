//! RMA receiving service — Guard → Mutation → Outbox for all RMA lifecycle operations.
//!
//! Invariants:
//! - Every disposition transition is validated by the state machine
//! - Every mutation writes its event to the outbox atomically in the same tx
//! - Idempotent receive via idempotency_key (unique per tenant)
//! - All queries filter by tenant_id

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

use super::state_machine::{validate_disposition, DispositionTransitionError};
use super::types::DispositionStatus;
use crate::db::rma_repo::RmaRepo;
use crate::outbox;

// ── Domain model ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct RmaReceipt {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub rma_id: String,
    pub customer_id: Uuid,
    pub condition_notes: Option<String>,
    pub disposition_status: String,
    pub idempotency_key: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct RmaReceiptItem {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub rma_receipt_id: Uuid,
    pub sku: String,
    pub qty: i64,
    pub condition_notes: Option<String>,
    pub created_at: DateTime<Utc>,
}

// ── Request types ─────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ReceiveRmaRequest {
    pub rma_id: String,
    pub customer_id: Uuid,
    pub condition_notes: Option<String>,
    pub items: Vec<RmaItemInput>,
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RmaItemInput {
    pub sku: String,
    pub qty: i64,
    pub condition_notes: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DispositionRequest {
    pub disposition_status: String,
}

// ── Errors ────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum RmaError {
    #[error("RMA receipt not found")]
    NotFound,

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Transition error: {0}")]
    Transition(#[from] DispositionTransitionError),

    #[error("RMA must have at least one item")]
    NoItems,

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ── Event type constants ─────────────────────────────────────

pub mod subjects {
    pub const RMA_RECEIVED: &str = "sr.rma.received";
    pub const RMA_DISPOSITION_CHANGED: &str = "sr.rma.disposition_changed";
}

// ── Service ──────────────────────────────────────────────────

pub struct RmaService;

impl RmaService {
    /// Receive an RMA: create receipt + items atomically with outbox event.
    ///
    /// Idempotent: if `idempotency_key` matches an existing receipt within the
    /// same tenant, returns the existing receipt without creating a duplicate.
    pub async fn receive(
        pool: &PgPool,
        tenant_id: Uuid,
        req: &ReceiveRmaRequest,
    ) -> Result<RmaReceipt, RmaError> {
        if req.items.is_empty() {
            return Err(RmaError::NoItems);
        }

        for (i, item) in req.items.iter().enumerate() {
            if item.qty <= 0 {
                return Err(RmaError::Validation(format!(
                    "item[{i}] qty must be > 0, got {}",
                    item.qty
                )));
            }
        }

        let mut tx = pool.begin().await?;

        // ── Idempotency check ──
        if let Some(ref key) = req.idempotency_key {
            if let Some(existing) =
                RmaRepo::find_by_idempotency_key_tx(&mut tx, tenant_id, key).await?
            {
                tx.commit().await?;
                return Ok(existing);
            }
        }

        // ── Mutation: insert receipt ──
        let receipt = RmaRepo::insert_receipt_tx(
            &mut tx,
            tenant_id,
            &req.rma_id,
            req.customer_id,
            req.condition_notes.as_deref(),
            req.idempotency_key.as_deref(),
        )
        .await?;

        // ── Mutation: insert items ──
        for item in &req.items {
            RmaRepo::insert_item_tx(
                &mut tx,
                tenant_id,
                receipt.id,
                &item.sku,
                item.qty,
                item.condition_notes.as_deref(),
            )
            .await?;
        }

        // ── Outbox ──
        let event_payload = serde_json::json!({
            "rma_receipt_id": receipt.id,
            "tenant_id": tenant_id,
            "rma_id": &req.rma_id,
            "customer_id": req.customer_id,
            "disposition_status": "received",
            "item_count": req.items.len(),
        });

        let event_id = Uuid::new_v4();
        outbox::enqueue_event_tx(
            &mut tx,
            event_id,
            subjects::RMA_RECEIVED,
            "rma_receipt",
            &receipt.id.to_string(),
            &tenant_id.to_string(),
            &event_payload,
        )
        .await?;

        tx.commit().await?;
        Ok(receipt)
    }

    /// Transition an RMA receipt's disposition status.
    ///
    /// Guard → Mutation → Outbox, all within a single transaction.
    pub async fn transition_disposition(
        pool: &PgPool,
        rma_receipt_id: Uuid,
        tenant_id: Uuid,
        req: &DispositionRequest,
    ) -> Result<RmaReceipt, RmaError> {
        let to = DispositionStatus::from_str_value(&req.disposition_status)
            .map_err(|e| RmaError::Validation(e.to_string()))?;

        let mut tx = pool.begin().await?;

        // ── Guard: load current state with row lock ──
        let current = RmaRepo::get_for_update_tx(&mut tx, rma_receipt_id, tenant_id)
            .await?
            .ok_or(RmaError::NotFound)?;

        let from = DispositionStatus::from_str_value(&current.disposition_status)
            .map_err(|e| RmaError::Validation(e.to_string()))?;

        // ── Guard: validate state machine transition ──
        validate_disposition(from, to)?;

        // ── Mutation ──
        let updated =
            RmaRepo::update_disposition_tx(&mut tx, rma_receipt_id, tenant_id, to.as_str()).await?;

        // ── Outbox ──
        let event_payload = serde_json::json!({
            "rma_receipt_id": rma_receipt_id,
            "tenant_id": tenant_id,
            "from_disposition": from.as_str(),
            "to_disposition": to.as_str(),
        });

        let event_id = Uuid::new_v4();
        outbox::enqueue_event_tx(
            &mut tx,
            event_id,
            subjects::RMA_DISPOSITION_CHANGED,
            "rma_receipt",
            &rma_receipt_id.to_string(),
            &tenant_id.to_string(),
            &event_payload,
        )
        .await?;

        tx.commit().await?;
        Ok(updated)
    }

    /// Find an RMA receipt by ID within a tenant.
    pub async fn find_by_id(
        pool: &PgPool,
        id: Uuid,
        tenant_id: Uuid,
    ) -> Result<Option<RmaReceipt>, RmaError> {
        RmaRepo::get_receipt(pool, id, tenant_id)
            .await
            .map_err(RmaError::Database)
    }

    /// List items for an RMA receipt.
    pub async fn list_items(
        pool: &PgPool,
        rma_receipt_id: Uuid,
        tenant_id: Uuid,
    ) -> Result<Vec<RmaReceiptItem>, RmaError> {
        RmaRepo::get_items(pool, rma_receipt_id, tenant_id)
            .await
            .map_err(RmaError::Database)
    }
}
