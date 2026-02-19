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

        let (ingest_id, _is_duplicate) = match ingest_result {
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

// ============================================================================
// Integrated Tests (real DB — requires DATABASE_URL env var)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use serde_json::json;

    const TEST_APP: &str = "test-webhook-svc";

    fn test_db_url() -> String {
        std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://integrations_user:integrations_pass@localhost:5449/integrations_db"
                .to_string()
        })
    }

    async fn test_pool() -> PgPool {
        let pool = sqlx::PgPool::connect(&test_db_url())
            .await
            .expect("Failed to connect to integrations test database");
        sqlx::migrate!("./db/migrations")
            .run(&pool)
            .await
            .expect("Migrations failed");
        pool
    }

    async fn cleanup(pool: &PgPool) {
        sqlx::query("DELETE FROM integrations_outbox WHERE app_id = $1")
            .bind(TEST_APP)
            .execute(pool)
            .await
            .ok();
        sqlx::query("DELETE FROM integrations_webhook_ingest WHERE app_id = $1")
            .bind(TEST_APP)
            .execute(pool)
            .await
            .ok();
    }

    fn internal_req(idempotency_key: Option<&str>, event_type: Option<&str>) -> IngestWebhookRequest {
        IngestWebhookRequest {
            app_id: TEST_APP.to_string(),
            system: "internal".to_string(),
            event_type: event_type.map(str::to_string),
            idempotency_key: idempotency_key.map(str::to_string),
            raw_payload: json!({ "data": "test" }),
            headers: std::collections::HashMap::new(),
        }
    }

    /// Webhook endpoint persists raw payload and metadata.
    #[tokio::test]
    #[serial]
    async fn test_webhook_ingest_persists_payload() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let svc = WebhookService::new(pool.clone());
        let body = br#"{"data":"test"}"#;
        let req = internal_req(Some("evt-persist-001"), Some("my.custom.event"));
        let result = svc.ingest(req, body).await.expect("ingest failed");

        assert!(!result.is_duplicate);
        assert!(result.ingest_id > 0);

        // Verify row was written
        let row: Option<(String, Option<String>, bool)> = sqlx::query_as(
            "SELECT system, event_type, processed_at IS NOT NULL
             FROM integrations_webhook_ingest WHERE id = $1",
        )
        .bind(result.ingest_id)
        .fetch_optional(&pool)
        .await
        .expect("query failed");

        let (system, event_type, is_processed) = row.expect("row should exist");
        assert_eq!(system, "internal");
        assert_eq!(event_type.as_deref(), Some("my.custom.event"));
        assert!(is_processed);

        cleanup(&pool).await;
    }

    /// Idempotency prevents replay double-processing.
    #[tokio::test]
    #[serial]
    async fn test_webhook_idempotency_prevents_duplicate() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let svc = WebhookService::new(pool.clone());
        let body = b"{}";

        let req1 = internal_req(Some("evt-dedup-001"), None);
        let req2 = internal_req(Some("evt-dedup-001"), None);

        let r1 = svc.ingest(req1, body).await.expect("first ingest failed");
        assert!(!r1.is_duplicate);

        let r2 = svc.ingest(req2, body).await.expect("second ingest failed");
        assert!(r2.is_duplicate);
        assert_eq!(r1.ingest_id, r2.ingest_id);

        // Only one row in DB
        let count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM integrations_webhook_ingest
             WHERE app_id = $1 AND idempotency_key = 'evt-dedup-001'",
        )
        .bind(TEST_APP)
        .fetch_one(&pool)
        .await
        .expect("count query failed");
        assert_eq!(count.0, 1);

        cleanup(&pool).await;
    }

    /// Routed domain event emitted via outbox (EventEnvelope compliant).
    #[tokio::test]
    #[serial]
    async fn test_webhook_routed_event_in_outbox() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let svc = WebhookService::new(pool.clone());
        let body = b"{}";
        let req = internal_req(Some("evt-route-001"), Some("my.custom.event"));
        let result = svc.ingest(req, body).await.expect("ingest failed");

        assert!(!result.is_duplicate);

        // Check outbox has at least one event for this ingest
        let outbox_rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT event_type, aggregate_id FROM integrations_outbox
             WHERE app_id = $1 AND aggregate_type = 'webhook'",
        )
        .bind(TEST_APP)
        .fetch_all(&pool)
        .await
        .expect("outbox query failed");

        assert!(!outbox_rows.is_empty(), "outbox should contain events");
        let event_types: Vec<&str> = outbox_rows.iter().map(|(et, _)| et.as_str()).collect();
        assert!(
            event_types.contains(&"webhook.received"),
            "webhook.received must be in outbox"
        );

        cleanup(&pool).await;
    }

    /// Signature verification rejects unknown systems.
    #[tokio::test]
    #[serial]
    async fn test_webhook_unsupported_system_rejected() {
        let pool = test_pool().await;
        let svc = WebhookService::new(pool.clone());

        let req = IngestWebhookRequest {
            app_id: TEST_APP.to_string(),
            system: "unknown-system".to_string(),
            event_type: None,
            idempotency_key: None,
            raw_payload: json!({}),
            headers: std::collections::HashMap::new(),
        };

        let result = svc.ingest(req, b"{}").await;
        assert!(matches!(result, Err(WebhookError::UnsupportedSystem { .. })));
    }
}
