//! Webhook ingest service — Guard → Mutation → Outbox pattern.
//!
//! Processing order:
//! 1. Guard: Signature verification (stateless, no DB I/O).
//! 2. Mutation: INSERT raw payload into `integrations_webhook_ingest`.
//!    - Duplicate idempotency_key → return `IngestResult { is_duplicate: true }`.
//! 3. Outbox (atomic): Emit `webhook.received` event.
//! 4. Routing: Map to domain event type and emit `webhook.routed` (if mapped).

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use crate::events::{build_webhook_received_envelope, WebhookReceivedPayload, EVENT_TYPE_WEBHOOK_RECEIVED};
use crate::outbox::enqueue_event_tx;

use super::models::{IngestResult, IngestWebhookRequest, WebhookError};
use super::routing::{emit_routed_event_tx, map_to_domain_event};
use super::verify::verify_signature;

pub struct WebhookService {
    pool: PgPool,
}

impl WebhookService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Ingest an inbound webhook.
    ///
    /// # Guard
    /// Signature verification is performed using raw bytes from the HTTP body
    /// **before** any database operations.
    ///
    /// # Idempotency
    /// If `idempotency_key` is set and a row with the same
    /// `(app_id, system, idempotency_key)` already exists, returns
    /// `IngestResult { is_duplicate: true }` without re-emitting events.
    pub async fn ingest(
        &self,
        req: IngestWebhookRequest,
        raw_body: &[u8],
    ) -> Result<IngestResult, WebhookError> {
        // ── 1. Guard: signature verification ────────────────────────────────
        verify_signature(&req.system, &req.headers, raw_body)?;

        // ── 2. Begin transaction ─────────────────────────────────────────────
        let mut tx = self.pool.begin().await?;

        // ── 3. Insert raw payload (dedup via UNIQUE constraint) ──────────────
        let headers_json = serde_json::to_value(&req.headers)
            .map_err(|e| WebhookError::Serialization(e.to_string()))?;

        let ingest_result = sqlx::query_as::<_, (i64,)>(
            r#"
            INSERT INTO integrations_webhook_ingest
                (app_id, system, event_type, raw_payload, headers, received_at, idempotency_key)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            ON CONFLICT ON CONSTRAINT integrations_webhook_ingest_dedup DO NOTHING
            RETURNING id
            "#,
        )
        .bind(&req.app_id)
        .bind(&req.system)
        .bind(&req.event_type)
        .bind(&req.raw_payload)
        .bind(&headers_json)
        .bind(Utc::now())
        .bind(&req.idempotency_key)
        .fetch_optional(&mut *tx)
        .await?;

        let (ingest_id, is_duplicate) = match ingest_result {
            Some((id,)) => (id, false),
            None => {
                // Duplicate — look up the existing record's id
                let existing = if req.idempotency_key.is_some() {
                    sqlx::query_as::<_, (i64,)>(
                        "SELECT id FROM integrations_webhook_ingest
                         WHERE app_id = $1 AND system = $2 AND idempotency_key = $3",
                    )
                    .bind(&req.app_id)
                    .bind(&req.system)
                    .bind(&req.idempotency_key)
                    .fetch_optional(&mut *tx)
                    .await?
                } else {
                    None
                };

                tx.rollback().await?;

                let id = existing.map(|(id,)| id).unwrap_or(0);
                return Ok(IngestResult {
                    ingest_id: id,
                    is_duplicate: true,
                });
            }
        };

        // ── 4. Outbox: emit webhook.received ─────────────────────────────────
        let correlation_id = Uuid::new_v4().to_string();
        let received_event_id = Uuid::new_v4();

        let received_envelope = build_webhook_received_envelope(
            received_event_id,
            req.app_id.clone(),
            correlation_id.clone(),
            None,
            WebhookReceivedPayload {
                ingest_id,
                system: req.system.clone(),
                event_type: req.event_type.clone(),
                idempotency_key: req.idempotency_key.clone(),
                received_at: Utc::now(),
            },
        );

        enqueue_event_tx(
            &mut tx,
            received_event_id,
            EVENT_TYPE_WEBHOOK_RECEIVED,
            "webhook",
            &ingest_id.to_string(),
            &req.app_id,
            &received_envelope,
        )
        .await?;

        // ── 5. Mark as processed + route domain event ─────────────────────────
        if let Some(domain_event_type) = map_to_domain_event(
            &req.system,
            req.event_type.as_deref(),
        ) {
            emit_routed_event_tx(
                &mut tx,
                ingest_id,
                &req.app_id,
                &req.system,
                req.event_type.as_deref(),
                &domain_event_type,
                &correlation_id,
            )
            .await?;
        }

        // Mark ingest record as processed
        sqlx::query(
            "UPDATE integrations_webhook_ingest SET processed_at = $1 WHERE id = $2",
        )
        .bind(Utc::now())
        .bind(ingest_id)
        .execute(&mut *tx)
        .await?;

        // ── 6. Commit ─────────────────────────────────────────────────────────
        tx.commit().await?;

        Ok(IngestResult {
            ingest_id,
            is_duplicate: false,
        })
    }
}
