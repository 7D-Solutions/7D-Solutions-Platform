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
use super::repo;
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

        let batch_result =
            repo::insert_batch_ingest(&mut tx, raw_payload, &headers_json, Utc::now(), &body_hash)
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
            let rows = repo::batch_resolve_realms(&mut tx, &unique_realms).await?;
            rows.into_iter()
                .collect::<std::collections::HashMap<String, String>>()
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
        repo::mark_ingest_processed(&mut tx, batch_ingest_id, Utc::now()).await?;

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

        let event_ingest = repo::insert_event_ingest(
            tx,
            &app_id,
            &event.event_type,
            &event_payload,
            Utc::now(),
            &event.id,
        )
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
        repo::mark_ingest_processed(tx, event_ingest_id, Utc::now()).await?;

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

// ============================================================================
// Tests (real DB — requires DATABASE_URL env var)
// ============================================================================

#[cfg(test)]
#[path = "qbo_normalizer_tests.rs"]
mod tests;
