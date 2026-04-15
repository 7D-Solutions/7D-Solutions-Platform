//! EDI transaction service — Guard → Mutation → Outbox pattern.
//!
//! Manages the lifecycle of EDI transactions:
//! - ingest: new inbound document in "ingested" status (idempotent via idempotency_key)
//! - create_outbound: new outbound record in "created" status
//! - transition: advance through the validation pipeline
//! - get / list: tenant-scoped reads

use sqlx::PgPool;
use uuid::Uuid;

use crate::events::{
    build_edi_transaction_created_envelope, build_edi_transaction_status_changed_envelope,
    EdiTransactionCreatedPayload, EdiTransactionStatusChangedPayload,
    EVENT_TYPE_EDI_TRANSACTION_CREATED, EVENT_TYPE_EDI_TRANSACTION_STATUS_CHANGED,
};
use crate::outbox::enqueue_event_tx;

use super::guards::{validate_create_outbound, validate_ingest, validate_transition};
use super::models::{
    CreateOutboundEdiRequest, EdiTransaction, EdiTransactionError, IngestEdiRequest,
    TransitionEdiRequest, DIRECTION_INBOUND, DIRECTION_OUTBOUND, STATUS_CREATED, STATUS_INGESTED,
};
use super::repo;

pub struct EdiTransactionService {
    pool: PgPool,
}

impl EdiTransactionService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    // ========================================================================
    // Reads
    // ========================================================================

    /// Get a single transaction by ID, scoped to tenant.
    pub async fn get(
        &self,
        tenant_id: &str,
        transaction_id: Uuid,
    ) -> Result<Option<EdiTransaction>, EdiTransactionError> {
        Ok(repo::get_by_id(&self.pool, tenant_id, transaction_id).await?)
    }

    /// List all transactions for a tenant.
    pub async fn list(&self, tenant_id: &str) -> Result<Vec<EdiTransaction>, EdiTransactionError> {
        Ok(repo::list_by_tenant(&self.pool, tenant_id).await?)
    }

    // ========================================================================
    // Ingest (inbound) — Guard → Mutation → Outbox
    // ========================================================================

    /// Ingest a new inbound EDI document. Idempotent: if a transaction with
    /// the same (tenant_id, idempotency_key) exists, the existing record is returned.
    pub async fn ingest(
        &self,
        req: IngestEdiRequest,
    ) -> Result<EdiTransaction, EdiTransactionError> {
        // ── Guard ────────────────────────────────────────────────────
        validate_ingest(&req)?;

        let mut tx = self.pool.begin().await?;

        // Idempotency check
        if let Some(ref key) = req.idempotency_key {
            let existing = repo::find_by_idempotency_key(&mut tx, &req.tenant_id, key).await?;

            if let Some(txn) = existing {
                tx.rollback().await?;
                return Ok(txn);
            }
        }

        // ── Mutation ─────────────────────────────────────────────────
        let txn_id = Uuid::new_v4();

        let txn = repo::insert_inbound(
            &mut tx,
            txn_id,
            &req.tenant_id,
            req.transaction_type.trim(),
            req.version.trim(),
            DIRECTION_INBOUND,
            &req.raw_payload,
            STATUS_INGESTED,
            &req.idempotency_key,
        )
        .await?;

        // ── Outbox ───────────────────────────────────────────────────
        let event_id = Uuid::new_v4();
        let correlation_id = Uuid::new_v4().to_string();

        let envelope = build_edi_transaction_created_envelope(
            event_id,
            req.tenant_id.clone(),
            correlation_id,
            None,
            EdiTransactionCreatedPayload {
                transaction_id: txn.id,
                tenant_id: req.tenant_id.clone(),
                transaction_type: txn.transaction_type.clone(),
                version: txn.version.clone(),
                direction: txn.direction.clone(),
                validation_status: txn.validation_status.clone(),
                created_at: txn.created_at,
            },
        );

        enqueue_event_tx(
            &mut tx,
            event_id,
            EVENT_TYPE_EDI_TRANSACTION_CREATED,
            "edi_transaction",
            &txn.id.to_string(),
            &req.tenant_id,
            &envelope,
        )
        .await?;

        tx.commit().await?;
        Ok(txn)
    }

    // ========================================================================
    // Create outbound — Guard → Mutation → Outbox
    // ========================================================================

    /// Create a new outbound EDI record. Idempotent via idempotency_key.
    pub async fn create_outbound(
        &self,
        req: CreateOutboundEdiRequest,
    ) -> Result<EdiTransaction, EdiTransactionError> {
        // ── Guard ────────────────────────────────────────────────────
        validate_create_outbound(&req)?;

        let mut tx = self.pool.begin().await?;

        // Idempotency check
        if let Some(ref key) = req.idempotency_key {
            let existing = repo::find_by_idempotency_key(&mut tx, &req.tenant_id, key).await?;

            if let Some(txn) = existing {
                tx.rollback().await?;
                return Ok(txn);
            }
        }

        // ── Mutation ─────────────────────────────────────────────────
        let txn_id = Uuid::new_v4();

        let txn = repo::insert_outbound(
            &mut tx,
            txn_id,
            &req.tenant_id,
            req.transaction_type.trim(),
            req.version.trim(),
            DIRECTION_OUTBOUND,
            &req.parsed_payload,
            STATUS_CREATED,
            &req.idempotency_key,
        )
        .await?;

        // ── Outbox ───────────────────────────────────────────────────
        let event_id = Uuid::new_v4();
        let correlation_id = Uuid::new_v4().to_string();

        let envelope = build_edi_transaction_created_envelope(
            event_id,
            req.tenant_id.clone(),
            correlation_id,
            None,
            EdiTransactionCreatedPayload {
                transaction_id: txn.id,
                tenant_id: req.tenant_id.clone(),
                transaction_type: txn.transaction_type.clone(),
                version: txn.version.clone(),
                direction: txn.direction.clone(),
                validation_status: txn.validation_status.clone(),
                created_at: txn.created_at,
            },
        );

        enqueue_event_tx(
            &mut tx,
            event_id,
            EVENT_TYPE_EDI_TRANSACTION_CREATED,
            "edi_transaction",
            &txn.id.to_string(),
            &req.tenant_id,
            &envelope,
        )
        .await?;

        tx.commit().await?;
        Ok(txn)
    }

    // ========================================================================
    // Transition — Guard → Mutation → Outbox
    // ========================================================================

    /// Advance a transaction through the validation pipeline.
    /// Validates the transition is legal for the transaction's direction.
    pub async fn transition(
        &self,
        req: TransitionEdiRequest,
    ) -> Result<EdiTransaction, EdiTransactionError> {
        let mut tx = self.pool.begin().await?;

        // Fetch + lock
        let existing = repo::fetch_for_update(&mut tx, req.transaction_id, &req.tenant_id)
            .await?
            .ok_or(EdiTransactionError::NotFound)?;

        // ── Guard ────────────────────────────────────────────────────
        let previous_status = existing.validation_status.clone();
        validate_transition(&previous_status, &existing.direction, &req)?;

        // ── Mutation ─────────────────────────────────────────────────
        let updated = repo::update_status(
            &mut tx,
            &req.new_status,
            &req.error_details,
            &req.parsed_payload,
            req.transaction_id,
            &req.tenant_id,
        )
        .await?;

        // ── Outbox ───────────────────────────────────────────────────
        let event_id = Uuid::new_v4();
        let correlation_id = Uuid::new_v4().to_string();

        let envelope = build_edi_transaction_status_changed_envelope(
            event_id,
            req.tenant_id.clone(),
            correlation_id,
            None,
            EdiTransactionStatusChangedPayload {
                transaction_id: updated.id,
                tenant_id: req.tenant_id.clone(),
                previous_status,
                new_status: req.new_status.clone(),
                error_details: req.error_details.clone(),
                changed_at: updated.updated_at,
            },
        );

        enqueue_event_tx(
            &mut tx,
            event_id,
            EVENT_TYPE_EDI_TRANSACTION_STATUS_CHANGED,
            "edi_transaction",
            &updated.id.to_string(),
            &req.tenant_id,
            &envelope,
        )
        .await?;

        tx.commit().await?;
        Ok(updated)
    }
}
