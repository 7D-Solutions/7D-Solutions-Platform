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

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::qbo::cdc::{
    comparable_fields, parse_last_updated_time, qbo_base_url, DbTokenProvider,
};
use crate::domain::qbo::client::QboClient;
use crate::domain::qbo::TokenProvider;
use crate::domain::sync::dedupe::{compute_comparable_hash, compute_fingerprint, truncate_to_millis};
use crate::domain::sync::detector;
use crate::domain::sync::observations;
use crate::events::{
    build_webhook_received_envelope, build_webhook_routed_envelope, WebhookReceivedPayload,
    WebhookRoutedPayload, EVENT_TYPE_WEBHOOK_RECEIVED, EVENT_TYPE_WEBHOOK_ROUTED,
};
use crate::outbox::enqueue_event_tx;

use super::models::WebhookError;
use super::repo;
use super::routing::{map_to_domain_event, qbo_entity_info};

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
    base_url: String,
}

impl QboNormalizer {
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            base_url: qbo_base_url(),
        }
    }

    pub fn new_with_base_url(pool: PgPool, base_url: String) -> Self {
        Self { pool, base_url }
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
        // Collect successfully-processed events for fetch-and-observe after commit.
        let mut to_observe: Vec<(QboCloudEvent, String, String)> = Vec::new();

        for event in &events {
            match self
                .process_single_event(&mut tx, event, &correlation_id, &realm_to_app)
                .await
            {
                Ok(true) => {
                    processed += 1;
                    if let Some(app_id) = realm_to_app.get(&event.intuit_account_id) {
                        to_observe.push((
                            event.clone(),
                            app_id.clone(),
                            event.intuit_account_id.clone(),
                        ));
                    }
                }
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

        // 5. Fetch-and-observe: write canonical observation rows outside the transaction.
        //    Failures are logged as warnings — the CDC poll will catch up if this fails.
        for (event, app_id, realm_id) in &to_observe {
            if let Err(e) = self.fetch_and_observe(event, app_id, realm_id).await {
                tracing::warn!(
                    event_id = %event.id,
                    event_type = %event.event_type,
                    app_id = %app_id,
                    error = %e,
                    "Webhook fetch-and-observe failed — CDC will cover"
                );
            }
        }

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

    /// Fetch the fresh entity from QBO (or build a synthetic tombstone payload for deletes)
    /// and write a canonical observation row.
    ///
    /// For delete events no API call is made — a tombstone observation is written using the
    /// event's `time` field as `last_updated_time`.  For all other events the entity is
    /// fetched via `GET /v3/company/{realm}/{entity}/{id}` and the response processed with
    /// the same fingerprint/comparable-hash logic used by CDC.
    ///
    /// A failure here is non-fatal: the outbox events are already committed and the CDC
    /// poll will eventually write the observation anyway.
    async fn fetch_and_observe(
        &self,
        event: &QboCloudEvent,
        app_id: &str,
        realm_id: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let (qbo_entity_type, obs_entity_type, is_delete) =
            match qbo_entity_info(&event.event_type) {
                Some(v) => v,
                None => return Ok(()),
            };

        let entity_id = match &event.intuit_entity_id {
            Some(id) if !id.is_empty() => id.clone(),
            _ => {
                tracing::warn!(
                    event_id = %event.id,
                    event_type = %event.event_type,
                    "QBO webhook missing intuitentityid — skipping observe"
                );
                return Ok(());
            }
        };

        if is_delete {
            let lut = truncate_to_millis(parse_event_time(&event.time));
            let tombstone = serde_json::json!({ "Id": &entity_id, "status": "Deleted" });
            let comparable = comparable_fields(&tombstone);
            let fingerprint = compute_fingerprint(None, Some(lut), &tombstone);
            let comparable_hash = compute_comparable_hash(&comparable, lut);

            let obs = observations::upsert_observation(
                &self.pool,
                app_id,
                "quickbooks",
                obs_entity_type,
                &entity_id,
                &fingerprint,
                lut,
                &comparable_hash,
                1,
                &tombstone,
                "webhook",
                true,
            )
            .await?;

            if let Err(e) = detector::run_detector(
                &self.pool,
                app_id,
                "quickbooks",
                obs_entity_type,
                &entity_id,
                &obs.fingerprint,
                &obs.comparable_hash,
                None,
                Some(tombstone.clone()),
            )
            .await
            {
                tracing::warn!(
                    entity_type = obs_entity_type,
                    entity_id = %entity_id,
                    error = %e,
                    "Detector error after webhook tombstone observation — conflict may be lost"
                );
            }
        } else {
            let tokens: Arc<dyn TokenProvider> = Arc::new(DbTokenProvider {
                pool: self.pool.clone(),
                app_id: app_id.to_string(),
            });
            let client = QboClient::new(&self.base_url, realm_id, tokens);
            let response = client.get_entity(qbo_entity_type, &entity_id).await?;

            let entity = &response[qbo_entity_type];
            if entity.is_null() {
                tracing::warn!(
                    entity_type = obs_entity_type,
                    entity_id = %entity_id,
                    "QBO GET response missing entity key — skipping observe"
                );
                return Ok(());
            }

            let lut = truncate_to_millis(parse_last_updated_time(entity));
            let sync_token = entity.get("SyncToken").and_then(|v| v.as_str());
            let comparable = comparable_fields(entity);
            let fingerprint = compute_fingerprint(sync_token, Some(lut), entity);
            let comparable_hash = compute_comparable_hash(&comparable, lut);

            let obs = observations::upsert_observation(
                &self.pool,
                app_id,
                "quickbooks",
                obs_entity_type,
                &entity_id,
                &fingerprint,
                lut,
                &comparable_hash,
                1,
                entity,
                "webhook",
                false,
            )
            .await?;

            if let Err(e) = detector::run_detector(
                &self.pool,
                app_id,
                "quickbooks",
                obs_entity_type,
                &entity_id,
                &obs.fingerprint,
                &obs.comparable_hash,
                None,
                Some(entity.clone()),
            )
            .await
            {
                tracing::warn!(
                    entity_type = obs_entity_type,
                    entity_id = %entity_id,
                    error = %e,
                    "Detector error after webhook observation — conflict may be lost"
                );
            }
        }

        Ok(())
    }
}

// ============================================================================
// Helpers
// ============================================================================

fn parse_event_time(time: &Option<String>) -> DateTime<Utc> {
    if let Some(ts_str) = time {
        if let Ok(ts) = ts_str.parse::<DateTime<Utc>>() {
            return ts;
        }
    }
    Utc::now()
}

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
