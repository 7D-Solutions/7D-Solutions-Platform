//! Outbound webhook service — Guard → Mutation → Outbox pattern.
//!
//! CRUD operations for tenant-scoped outbound webhooks, plus delivery logging.
//! Every mutation emits an event through the outbox for downstream consumers.

use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use crate::events::{
    build_outbound_webhook_created_envelope, build_outbound_webhook_deleted_envelope,
    build_outbound_webhook_updated_envelope, OutboundWebhookCreatedPayload,
    OutboundWebhookDeletedPayload, OutboundWebhookUpdatedPayload,
    EVENT_TYPE_OUTBOUND_WEBHOOK_CREATED, EVENT_TYPE_OUTBOUND_WEBHOOK_DELETED,
    EVENT_TYPE_OUTBOUND_WEBHOOK_UPDATED,
};
use crate::outbox::enqueue_event_tx;

use super::guards::{validate_create, validate_update};
use super::models::{
    CreateOutboundWebhookRequest, OutboundWebhook, OutboundWebhookDelivery, OutboundWebhookError,
    RecordDeliveryRequest, UpdateOutboundWebhookRequest,
};
use super::repo;

pub struct OutboundWebhookService {
    pool: PgPool,
}

impl OutboundWebhookService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    // ========================================================================
    // Reads
    // ========================================================================

    /// Get a single webhook by ID, scoped to tenant.
    pub async fn get(
        &self,
        tenant_id: &str,
        webhook_id: Uuid,
    ) -> Result<Option<OutboundWebhook>, OutboundWebhookError> {
        Ok(repo::get_by_id(&self.pool, tenant_id, webhook_id).await?)
    }

    /// List all webhooks for a tenant (non-deleted).
    pub async fn list(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<OutboundWebhook>, OutboundWebhookError> {
        Ok(repo::list_by_tenant(&self.pool, tenant_id).await?)
    }

    // ========================================================================
    // Create — Guard → Mutation → Outbox
    // ========================================================================

    /// Create a new outbound webhook. Returns the webhook and the raw signing
    /// secret (returned once, never stored).
    pub async fn create(
        &self,
        req: CreateOutboundWebhookRequest,
    ) -> Result<(OutboundWebhook, String), OutboundWebhookError> {
        // ── Guard ───────────────────────────────────────────────────────
        validate_create(&req)?;

        // Generate signing secret
        let raw_secret = format!("whsec_{}", Uuid::new_v4().as_simple());
        let secret_hash = hex::encode(Sha256::digest(raw_secret.as_bytes()));

        let event_types_json = serde_json::to_value(&req.event_types)
            .map_err(|e| OutboundWebhookError::Validation(format!("invalid event_types: {}", e)))?;

        let webhook_id = Uuid::new_v4();

        // ── Mutation (transaction) ──────────────────────────────────────
        let mut tx = self.pool.begin().await?;

        // Check idempotency: if key exists, return the existing webhook
        if let Some(ref key) = req.idempotency_key {
            let existing =
                repo::find_by_idempotency_key(&mut tx, &req.tenant_id, key).await?;

            if let Some(wh) = existing {
                tx.rollback().await?;
                return Ok((wh, String::new()));
            }
        }

        let webhook = repo::insert(
            &mut tx,
            webhook_id,
            &req.tenant_id,
            &req.url,
            &event_types_json,
            &secret_hash,
            &req.idempotency_key,
            &req.description,
        )
        .await?;

        // ── Outbox ──────────────────────────────────────────────────────
        let event_id = Uuid::new_v4();
        let correlation_id = Uuid::new_v4().to_string();

        let envelope = build_outbound_webhook_created_envelope(
            event_id,
            req.tenant_id.clone(),
            correlation_id,
            None,
            OutboundWebhookCreatedPayload {
                webhook_id: webhook.id,
                tenant_id: req.tenant_id.clone(),
                url: req.url.clone(),
                event_types: req.event_types.clone(),
            },
        );

        enqueue_event_tx(
            &mut tx,
            event_id,
            EVENT_TYPE_OUTBOUND_WEBHOOK_CREATED,
            "outbound_webhook",
            &webhook.id.to_string(),
            &req.tenant_id,
            &envelope,
        )
        .await?;

        tx.commit().await?;

        Ok((webhook, raw_secret))
    }

    // ========================================================================
    // Update — Guard → Mutation → Outbox
    // ========================================================================

    pub async fn update(
        &self,
        req: UpdateOutboundWebhookRequest,
    ) -> Result<OutboundWebhook, OutboundWebhookError> {
        // ── Guard ───────────────────────────────────────────────────────
        validate_update(&req)?;

        let mut tx = self.pool.begin().await?;

        // Fetch existing (scoped to tenant)
        let existing = repo::fetch_for_update(&mut tx, req.id, &req.tenant_id)
            .await?
            .ok_or(OutboundWebhookError::NotFound)?;

        // ── Mutation ────────────────────────────────────────────────────
        let new_url = req.url.as_deref().unwrap_or(&existing.url);
        let new_event_types = match &req.event_types {
            Some(types) => serde_json::to_value(types).map_err(|e| {
                OutboundWebhookError::Validation(format!("invalid event_types: {}", e))
            })?,
            None => existing.event_types.clone(),
        };
        let new_status = req.status.as_deref().unwrap_or(&existing.status);
        let new_description = match &req.description {
            Some(d) => Some(d.as_str()),
            None => existing.description.as_deref(),
        };

        let updated = repo::update_fields(
            &mut tx,
            new_url,
            &new_event_types,
            new_status,
            new_description,
            req.id,
            &req.tenant_id,
        )
        .await?;

        // ── Outbox ──────────────────────────────────────────────────────
        let event_id = Uuid::new_v4();
        let correlation_id = Uuid::new_v4().to_string();

        let envelope = build_outbound_webhook_updated_envelope(
            event_id,
            req.tenant_id.clone(),
            correlation_id,
            None,
            OutboundWebhookUpdatedPayload {
                webhook_id: updated.id,
                tenant_id: req.tenant_id.clone(),
                url: updated.url.clone(),
                status: updated.status.clone(),
            },
        );

        enqueue_event_tx(
            &mut tx,
            event_id,
            EVENT_TYPE_OUTBOUND_WEBHOOK_UPDATED,
            "outbound_webhook",
            &updated.id.to_string(),
            &req.tenant_id,
            &envelope,
        )
        .await?;

        tx.commit().await?;

        Ok(updated)
    }

    // ========================================================================
    // Delete (soft) — Guard → Mutation → Outbox
    // ========================================================================

    pub async fn delete(
        &self,
        tenant_id: &str,
        webhook_id: Uuid,
    ) -> Result<(), OutboundWebhookError> {
        if tenant_id.is_empty() {
            return Err(OutboundWebhookError::Validation(
                "tenant_id is required".into(),
            ));
        }

        let mut tx = self.pool.begin().await?;

        let result = repo::soft_delete(&mut tx, webhook_id, tenant_id).await?;

        if result.rows_affected() == 0 {
            return Err(OutboundWebhookError::NotFound);
        }

        // ── Outbox ──────────────────────────────────────────────────────
        let event_id = Uuid::new_v4();
        let correlation_id = Uuid::new_v4().to_string();

        let envelope = build_outbound_webhook_deleted_envelope(
            event_id,
            tenant_id.to_string(),
            correlation_id,
            None,
            OutboundWebhookDeletedPayload {
                webhook_id,
                tenant_id: tenant_id.to_string(),
            },
        );

        enqueue_event_tx(
            &mut tx,
            event_id,
            EVENT_TYPE_OUTBOUND_WEBHOOK_DELETED,
            "outbound_webhook",
            &webhook_id.to_string(),
            tenant_id,
            &envelope,
        )
        .await?;

        tx.commit().await?;

        Ok(())
    }

    // ========================================================================
    // Delivery logging
    // ========================================================================

    /// Record a delivery attempt (success or failure).
    pub async fn record_delivery(
        &self,
        req: RecordDeliveryRequest,
    ) -> Result<OutboundWebhookDelivery, OutboundWebhookError> {
        Ok(repo::insert_delivery(&self.pool, &req).await?)
    }

    /// List deliveries for a specific webhook (tenant-scoped).
    pub async fn list_deliveries(
        &self,
        tenant_id: &str,
        webhook_id: Uuid,
    ) -> Result<Vec<OutboundWebhookDelivery>, OutboundWebhookError> {
        Ok(repo::list_deliveries(&self.pool, tenant_id, webhook_id).await?)
    }
}
