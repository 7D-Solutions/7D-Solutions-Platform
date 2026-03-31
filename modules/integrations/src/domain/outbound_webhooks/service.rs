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
        let row = sqlx::query_as::<_, OutboundWebhook>(
            r#"SELECT id, tenant_id, url, event_types, signing_secret_hash,
                      status, idempotency_key, description, created_at, updated_at, deleted_at
               FROM integrations_outbound_webhooks
               WHERE id = $1 AND tenant_id = $2 AND deleted_at IS NULL"#,
        )
        .bind(webhook_id)
        .bind(tenant_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// List all webhooks for a tenant (non-deleted).
    pub async fn list(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<OutboundWebhook>, OutboundWebhookError> {
        let rows = sqlx::query_as::<_, OutboundWebhook>(
            r#"SELECT id, tenant_id, url, event_types, signing_secret_hash,
                      status, idempotency_key, description, created_at, updated_at, deleted_at
               FROM integrations_outbound_webhooks
               WHERE tenant_id = $1 AND deleted_at IS NULL
               ORDER BY created_at DESC"#,
        )
        .bind(tenant_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
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
            let existing = sqlx::query_as::<_, OutboundWebhook>(
                r#"SELECT id, tenant_id, url, event_types, signing_secret_hash,
                          status, idempotency_key, description, created_at, updated_at, deleted_at
                   FROM integrations_outbound_webhooks
                   WHERE tenant_id = $1 AND idempotency_key = $2 AND deleted_at IS NULL"#,
            )
            .bind(&req.tenant_id)
            .bind(key)
            .fetch_optional(&mut *tx)
            .await?;

            if let Some(wh) = existing {
                tx.rollback().await?;
                return Ok((wh, String::new()));
            }
        }

        let webhook = sqlx::query_as::<_, OutboundWebhook>(
            r#"INSERT INTO integrations_outbound_webhooks
                   (id, tenant_id, url, event_types, signing_secret_hash,
                    status, idempotency_key, description)
               VALUES ($1, $2, $3, $4, $5, 'active', $6, $7)
               RETURNING id, tenant_id, url, event_types, signing_secret_hash,
                         status, idempotency_key, description, created_at, updated_at, deleted_at"#,
        )
        .bind(webhook_id)
        .bind(&req.tenant_id)
        .bind(&req.url)
        .bind(&event_types_json)
        .bind(&secret_hash)
        .bind(&req.idempotency_key)
        .bind(&req.description)
        .fetch_one(&mut *tx)
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
        let existing = sqlx::query_as::<_, OutboundWebhook>(
            r#"SELECT id, tenant_id, url, event_types, signing_secret_hash,
                      status, idempotency_key, description, created_at, updated_at, deleted_at
               FROM integrations_outbound_webhooks
               WHERE id = $1 AND tenant_id = $2 AND deleted_at IS NULL
               FOR UPDATE"#,
        )
        .bind(req.id)
        .bind(&req.tenant_id)
        .fetch_optional(&mut *tx)
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

        let updated = sqlx::query_as::<_, OutboundWebhook>(
            r#"UPDATE integrations_outbound_webhooks
               SET url = $1, event_types = $2, status = $3, description = $4, updated_at = NOW()
               WHERE id = $5 AND tenant_id = $6 AND deleted_at IS NULL
               RETURNING id, tenant_id, url, event_types, signing_secret_hash,
                         status, idempotency_key, description, created_at, updated_at, deleted_at"#,
        )
        .bind(new_url)
        .bind(&new_event_types)
        .bind(new_status)
        .bind(new_description)
        .bind(req.id)
        .bind(&req.tenant_id)
        .fetch_one(&mut *tx)
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

        let result = sqlx::query(
            r#"UPDATE integrations_outbound_webhooks
               SET deleted_at = NOW(), status = 'disabled', updated_at = NOW()
               WHERE id = $1 AND tenant_id = $2 AND deleted_at IS NULL"#,
        )
        .bind(webhook_id)
        .bind(tenant_id)
        .execute(&mut *tx)
        .await?;

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
        let delivery = sqlx::query_as::<_, OutboundWebhookDelivery>(
            r#"INSERT INTO integrations_outbound_webhook_deliveries
                   (webhook_id, tenant_id, event_type, payload, status_code,
                    response_body, error_message, attempt_number, next_retry_at, delivered_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
               RETURNING id, webhook_id, tenant_id, event_type, payload, status_code,
                         response_body, error_message, attempt_number, next_retry_at,
                         delivered_at, created_at"#,
        )
        .bind(req.webhook_id)
        .bind(&req.tenant_id)
        .bind(&req.event_type)
        .bind(&req.payload)
        .bind(req.status_code)
        .bind(&req.response_body)
        .bind(&req.error_message)
        .bind(req.attempt_number)
        .bind(req.next_retry_at)
        .bind(req.delivered_at)
        .fetch_one(&self.pool)
        .await?;

        Ok(delivery)
    }

    /// List deliveries for a specific webhook (tenant-scoped).
    pub async fn list_deliveries(
        &self,
        tenant_id: &str,
        webhook_id: Uuid,
    ) -> Result<Vec<OutboundWebhookDelivery>, OutboundWebhookError> {
        let rows = sqlx::query_as::<_, OutboundWebhookDelivery>(
            r#"SELECT id, webhook_id, tenant_id, event_type, payload, status_code,
                      response_body, error_message, attempt_number, next_retry_at,
                      delivered_at, created_at
               FROM integrations_outbound_webhook_deliveries
               WHERE webhook_id = $1 AND tenant_id = $2
               ORDER BY created_at DESC"#,
        )
        .bind(webhook_id)
        .bind(tenant_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }
}
