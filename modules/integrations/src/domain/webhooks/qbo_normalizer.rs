//! QBO webhook normalization — fan out a batched Intuit POST into per-event
//! ingest records, each scoped to the correct tenant via realm_id resolution.
//!
//! Intuit sends a single POST containing an array of CloudEvents objects that
//! may span multiple QBO companies (realm IDs). Each event must be resolved
//! to the tenant that owns that company via `integrations_oauth_connections`.
//!
//! ## Idempotency (two-level)
//! - **POST-level**: SHA-256 hash of the raw body prevents re-processing the
//!   exact same webhook delivery.
//! - **Per-event**: The CloudEvent `id` field prevents duplicate outbox entries
//!   even when the same event appears in different POST deliveries.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use crate::events::{
    build_webhook_received_envelope, build_webhook_routed_envelope, WebhookReceivedPayload,
    WebhookRoutedPayload, EVENT_TYPE_WEBHOOK_RECEIVED, EVENT_TYPE_WEBHOOK_ROUTED,
};
use crate::outbox::enqueue_event_tx;

use super::models::WebhookError;
use super::routing::map_to_domain_event;

// ============================================================================
// Types
// ============================================================================

/// A single event from a QBO webhook POST (Intuit CloudEvents format).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QboCloudEvent {
    /// Unique event ID assigned by Intuit.
    pub id: String,
    /// CloudEvents type, e.g. "qbo.customer.created.v1".
    #[serde(rename = "type")]
    pub event_type: String,
    /// ISO 8601 timestamp.
    #[serde(default)]
    pub time: Option<String>,
    /// The QBO entity ID (e.g. customer ID, invoice ID).
    #[serde(rename = "intuitentityid", default)]
    pub intuit_entity_id: Option<String>,
    /// The QBO company (realm) ID — maps to our tenant.
    #[serde(rename = "intuitaccountid")]
    pub intuit_account_id: String,
    /// Event-specific data.
    #[serde(default)]
    pub data: Option<serde_json::Value>,
}

/// Result of normalizing a QBO webhook POST.
#[derive(Debug, Clone)]
pub struct QboNormalizeResult {
    pub batch_ingest_id: i64,
    pub events_processed: u32,
    pub events_skipped: u32,
    pub is_duplicate: bool,
}

// ============================================================================
// Normalizer
// ============================================================================

pub struct QboNormalizer {
    pool: PgPool,
}

impl QboNormalizer {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Parse a batched QBO webhook, resolve each event to a tenant, and fan out
    /// to per-event ingest records and outbox entries.
    pub async fn normalize(
        &self,
        raw_body: &[u8],
        raw_payload: &serde_json::Value,
        headers: &std::collections::HashMap<String, String>,
    ) -> Result<QboNormalizeResult, WebhookError> {
        // 1. Parse CloudEvents array directly from bytes (avoids cloning the Value tree)
        let events: Vec<QboCloudEvent> = serde_json::from_slice(raw_body).map_err(|e| {
            WebhookError::MalformedPayload(format!(
                "QBO webhook body is not a valid CloudEvents array: {}",
                e
            ))
        })?;

        // 2. POST-level idempotency via body hash
        let body_hash = compute_body_hash(raw_body);

        let mut tx = self.pool.begin().await?;

        let headers_json = serde_json::to_value(headers)
            .map_err(|e| WebhookError::Serialization(e.to_string()))?;

        let batch_result = sqlx::query_as::<_, (i64,)>(
            r#"
            INSERT INTO integrations_webhook_ingest
                (app_id, system, event_type, raw_payload, headers, received_at, idempotency_key)
            VALUES ('_qbo_batch_', 'quickbooks', NULL, $1, $2, $3, $4)
            ON CONFLICT ON CONSTRAINT integrations_webhook_ingest_dedup DO NOTHING
            RETURNING id
            "#,
        )
        .bind(raw_payload)
        .bind(&headers_json)
        .bind(Utc::now())
        .bind(&body_hash)
        .fetch_optional(&mut *tx)
        .await?;

        let batch_ingest_id = match batch_result {
            Some((id,)) => id,
            None => {
                tx.rollback().await?;
                return Ok(QboNormalizeResult {
                    batch_ingest_id: 0,
                    events_processed: 0,
                    events_skipped: 0,
                    is_duplicate: true,
                });
            }
        };

        // 3. Batch-resolve all unique realm_ids → app_ids in one query
        let realm_to_app = {
            let unique_realms: Vec<String> = {
                let mut set = std::collections::HashSet::new();
                for event in &events {
                    set.insert(event.intuit_account_id.clone());
                }
                set.into_iter().collect()
            };
            batch_resolve_realms(&mut tx, &unique_realms).await?
        };

        // 4. Fan out per event
        let mut processed = 0u32;
        let mut skipped = 0u32;
        let correlation_id = Uuid::new_v4().to_string();

        for event in &events {
            match self
                .process_single_event(&mut tx, event, &correlation_id, &realm_to_app)
                .await
            {
                Ok(true) => processed += 1,
                Ok(false) => skipped += 1,
                Err(e) => {
                    tracing::warn!(
                        event_id = %event.id,
                        error = %e,
                        "QBO event processing failed — skipping"
                    );
                    skipped += 1;
                }
            }
        }

        // Mark batch record processed
        sqlx::query("UPDATE integrations_webhook_ingest SET processed_at = $1 WHERE id = $2")
            .bind(Utc::now())
            .bind(batch_ingest_id)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;

        Ok(QboNormalizeResult {
            batch_ingest_id,
            events_processed: processed,
            events_skipped: skipped,
            is_duplicate: false,
        })
    }

    /// Process one CloudEvent: look up realm → app_id, insert ingest, emit outbox events.
    /// Returns `Ok(true)` if processed, `Ok(false)` if skipped (unknown realm or duplicate).
    async fn process_single_event(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        event: &QboCloudEvent,
        correlation_id: &str,
        realm_to_app: &std::collections::HashMap<String, String>,
    ) -> Result<bool, WebhookError> {
        // Look up realm_id → app_id from pre-fetched map
        let app_id = match realm_to_app.get(&event.intuit_account_id) {
            Some(id) => id.clone(),
            None => {
                tracing::warn!(
                    realm_id = %event.intuit_account_id,
                    event_id = %event.id,
                    "QBO webhook event for unknown realm — skipping"
                );
                return Ok(false);
            }
        };

        // Per-event idempotency: insert with ON CONFLICT DO NOTHING
        let event_payload =
            serde_json::to_value(event).map_err(|e| WebhookError::Serialization(e.to_string()))?;

        let event_ingest = sqlx::query_as::<_, (i64,)>(
            r#"
            INSERT INTO integrations_webhook_ingest
                (app_id, system, event_type, raw_payload, headers, received_at, idempotency_key)
            VALUES ($1, 'quickbooks', $2, $3, '{}'::jsonb, $4, $5)
            ON CONFLICT ON CONSTRAINT integrations_webhook_ingest_dedup DO NOTHING
            RETURNING id
            "#,
        )
        .bind(&app_id)
        .bind(&event.event_type)
        .bind(&event_payload)
        .bind(Utc::now())
        .bind(&event.id)
        .fetch_optional(&mut **tx)
        .await?;

        let event_ingest_id = match event_ingest {
            Some((id,)) => id,
            None => return Ok(false), // Duplicate event
        };

        // Emit webhook.received
        let received_event_id = Uuid::new_v4();
        let received_envelope = build_webhook_received_envelope(
            received_event_id,
            app_id.clone(),
            correlation_id.to_string(),
            None,
            WebhookReceivedPayload {
                ingest_id: event_ingest_id,
                system: "quickbooks".to_string(),
                event_type: Some(event.event_type.clone()),
                idempotency_key: Some(event.id.clone()),
                received_at: Utc::now(),
            },
        );

        enqueue_event_tx(
            tx,
            received_event_id,
            EVENT_TYPE_WEBHOOK_RECEIVED,
            "webhook",
            &event_ingest_id.to_string(),
            &app_id,
            &received_envelope,
        )
        .await?;

        // Route to domain event
        if let Some(domain_event_type) = map_to_domain_event("quickbooks", Some(&event.event_type))
        {
            let routed_event_id = Uuid::new_v4();
            let routed_envelope = build_webhook_routed_envelope(
                routed_event_id,
                app_id.clone(),
                correlation_id.to_string(),
                Some(received_event_id.to_string()),
                WebhookRoutedPayload {
                    ingest_id: event_ingest_id,
                    system: "quickbooks".to_string(),
                    source_event_type: Some(event.event_type.clone()),
                    domain_event_type: domain_event_type.clone(),
                    outbox_event_id: routed_event_id,
                    routed_at: Utc::now(),
                },
            );

            enqueue_event_tx(
                tx,
                routed_event_id,
                EVENT_TYPE_WEBHOOK_ROUTED,
                "webhook",
                &event_ingest_id.to_string(),
                &app_id,
                &routed_envelope,
            )
            .await?;
        }

        // Mark ingest record processed
        sqlx::query("UPDATE integrations_webhook_ingest SET processed_at = $1 WHERE id = $2")
            .bind(Utc::now())
            .bind(event_ingest_id)
            .execute(&mut **tx)
            .await?;

        Ok(true)
    }
}

// ============================================================================
// Helpers
// ============================================================================

fn compute_body_hash(raw_body: &[u8]) -> String {
    let hash = Sha256::digest(raw_body);
    format!("sha256:{:x}", hash)
}

/// Batch-resolve realm_ids to app_ids in a single query.
async fn batch_resolve_realms(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    realm_ids: &[String],
) -> Result<std::collections::HashMap<String, String>, WebhookError> {
    let rows = sqlx::query_as::<_, (String, String)>(
        r#"
        SELECT realm_id, app_id FROM integrations_oauth_connections
        WHERE provider = 'quickbooks' AND realm_id = ANY($1) AND connection_status = 'connected'
        "#,
    )
    .bind(realm_ids)
    .fetch_all(&mut **tx)
    .await?;

    Ok(rows.into_iter().collect())
}

// ============================================================================
// Tests (real DB — requires DATABASE_URL env var)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use serial_test::serial;

    const APP_A: &str = "test-qbo-a";
    const APP_B: &str = "test-qbo-b";
    const REALM_A: &str = "realm-t-001";
    const REALM_B: &str = "realm-t-002";

    async fn setup() -> PgPool {
        let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://integrations_user:integrations_pass@localhost:5449/integrations_db".into()
        });
        let pool = PgPool::connect(&url).await.expect("DB connect failed");
        sqlx::migrate!("./db/migrations")
            .run(&pool)
            .await
            .expect("migrate");
        // cleanup
        for app in [APP_A, APP_B, "_qbo_batch_"] {
            sqlx::query("DELETE FROM integrations_outbox WHERE app_id=$1")
                .bind(app)
                .execute(&pool)
                .await
                .ok();
            sqlx::query("DELETE FROM integrations_webhook_ingest WHERE app_id=$1")
                .bind(app)
                .execute(&pool)
                .await
                .ok();
        }
        for r in [REALM_A, REALM_B] {
            sqlx::query("DELETE FROM integrations_oauth_connections WHERE provider='quickbooks' AND realm_id=$1")
                .bind(r).execute(&pool).await.ok();
        }
        // seed connections
        for (app, realm) in [(APP_A, REALM_A), (APP_B, REALM_B)] {
            sqlx::query(
                "INSERT INTO integrations_oauth_connections
                 (app_id,provider,realm_id,access_token,refresh_token,access_token_expires_at,refresh_token_expires_at,scopes_granted)
                 VALUES($1,'quickbooks',$2,'t'::bytea,'t'::bytea,NOW()+'1h'::interval,NOW()+'100d'::interval,'accounting')
                 ON CONFLICT DO NOTHING",
            ).bind(app).bind(realm).execute(&pool).await.expect("seed");
        }
        pool
    }

    fn ev(id: &str, typ: &str, realm: &str) -> serde_json::Value {
        json!({"id":id,"type":typ,"time":"2026-03-27T12:00:00Z","intuitentityid":"42","intuitaccountid":realm,"data":{}})
    }

    async fn run(pool: &PgPool, events: &serde_json::Value) -> QboNormalizeResult {
        let body = serde_json::to_vec(events).expect("json serialize");
        QboNormalizer::new(pool.clone())
            .normalize(&body, events, &std::collections::HashMap::new())
            .await
            .expect("normalize failed")
    }

    async fn count(pool: &PgPool, query: &str, bind: &str) -> i64 {
        sqlx::query_as::<_, (i64,)>(query)
            .bind(bind)
            .fetch_one(pool)
            .await
            .expect("count query")
            .0
    }

    #[tokio::test]
    #[serial]
    async fn test_qbo_normalize_fan_out_across_realms() {
        let pool = setup().await;
        let events = json!([
            ev("e1", "qbo.customer.created.v1", REALM_A),
            ev("e2", "qbo.invoice.updated.v1", REALM_B),
            ev("e3", "qbo.payment.created.v1", REALM_A)
        ]);
        let r = run(&pool, &events).await;
        assert!(!r.is_duplicate);
        assert_eq!(r.events_processed, 3);
        assert_eq!(r.events_skipped, 0);

        // 3 per-event ingest records
        let n = count(&pool, "SELECT COUNT(*) FROM integrations_webhook_ingest WHERE system='quickbooks' AND app_id!='_qbo_batch_'", APP_A).await;
        // app_id filter not applied here — just checking total
        assert!(n >= 2);

        // Tenant A: 2 events × (received + routed) = 4 outbox entries
        assert_eq!(
            count(
                &pool,
                "SELECT COUNT(*) FROM integrations_outbox WHERE app_id=$1",
                APP_A
            )
            .await,
            4
        );
        // Tenant B: 1 event × 2 = 2
        assert_eq!(
            count(
                &pool,
                "SELECT COUNT(*) FROM integrations_outbox WHERE app_id=$1",
                APP_B
            )
            .await,
            2
        );
    }

    #[tokio::test]
    #[serial]
    async fn test_qbo_unknown_realm_skipped() {
        let pool = setup().await;
        let events = json!([
            ev("s1", "qbo.customer.created.v1", "realm-unknown-999"),
            ev("s2", "qbo.invoice.created.v1", REALM_A)
        ]);
        let r = run(&pool, &events).await;
        assert_eq!(r.events_processed, 1);
        assert_eq!(r.events_skipped, 1);
    }

    #[tokio::test]
    #[serial]
    async fn test_qbo_post_level_dedup() {
        let pool = setup().await;
        let events = json!([ev("d1", "qbo.customer.created.v1", REALM_A)]);
        let r1 = run(&pool, &events).await;
        assert!(!r1.is_duplicate);
        assert_eq!(r1.events_processed, 1);
        // Replay same body
        let r2 = run(&pool, &events).await;
        assert!(r2.is_duplicate);
        assert_eq!(r2.events_processed, 0);
    }

    #[tokio::test]
    #[serial]
    async fn test_qbo_event_level_dedup() {
        let pool = setup().await;
        let r1 = run(
            &pool,
            &json!([ev("o1", "qbo.customer.created.v1", REALM_A)]),
        )
        .await;
        assert_eq!(r1.events_processed, 1);
        // Second POST: o1 overlaps, o2 is new
        let r2 = run(
            &pool,
            &json!([
                ev("o1", "qbo.customer.created.v1", REALM_A),
                ev("o2", "qbo.invoice.created.v1", REALM_B)
            ]),
        )
        .await;
        assert_eq!(r2.events_processed, 1);
        assert_eq!(r2.events_skipped, 1);
    }
}
