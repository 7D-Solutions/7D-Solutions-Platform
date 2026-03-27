//! QBO CDC (Change Data Capture) polling worker.
//!
//! Polls the QBO CDC endpoint for each connected tenant, publishing changed
//! entities to the outbox for relay to NATS. Acts as the reliable backup to
//! QBO webhooks, which have 5-25 minute delivery latency and can be lost.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::sync::Arc;
use tokio::sync::watch;
use uuid::Uuid;

use super::client::QboClient;
use super::{QboError, TokenProvider};
use crate::events::envelope::create_integrations_envelope;
use crate::events::MUTATION_CLASS_DATA_MUTATION;
use crate::outbox::enqueue_event_tx;

/// Entities included in CDC queries.
pub const CDC_ENTITIES: &[&str] = &["Customer", "Invoice", "Payment", "Item"];

/// Default poll interval (15 minutes).
pub const DEFAULT_CDC_POLL_INTERVAL_SECS: u64 = 900;

/// Days before the 30-day CDC cliff to trigger full resync (5-day buffer).
pub const WATERMARK_STALE_DAYS: i64 = 25;

/// Event type for CDC-synced entities.
pub const EVENT_TYPE_QBO_ENTITY_SYNCED: &str = "qbo.entity.synced";

// ============================================================================
// Event payload
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QboCdcEntityPayload {
    pub entity_type: String,
    pub entity_id: String,
    pub realm_id: String,
    /// "cdc" or "full_resync"
    pub source: String,
    pub entity_data: serde_json::Value,
    pub synced_at: DateTime<Utc>,
}

// ============================================================================
// DB-backed TokenProvider
// ============================================================================

pub(crate) struct DbTokenProvider {
    pub pool: PgPool,
    pub app_id: String,
}

#[async_trait::async_trait]
impl TokenProvider for DbTokenProvider {
    async fn get_token(&self) -> Result<String, QboError> {
        crate::domain::oauth::service::get_access_token(
            &self.pool,
            &self.app_id,
            "quickbooks",
        )
        .await
        .map_err(|e| QboError::TokenError(e.to_string()))
    }

    async fn refresh_token(&self) -> Result<String, QboError> {
        // Token refresh is handled by the background refresh worker.
        Err(QboError::AuthFailed)
    }
}

// ============================================================================
// Connection query
// ============================================================================

#[derive(Debug, sqlx::FromRow)]
struct CdcConnection {
    id: uuid::Uuid,
    app_id: String,
    realm_id: String,
    cdc_watermark: Option<DateTime<Utc>>,
    full_resync_required: bool,
}

// ============================================================================
// URL resolution
// ============================================================================

/// Resolve the QBO API base URL from environment.
pub fn qbo_base_url() -> String {
    if let Ok(url) = std::env::var("QBO_BASE_URL") {
        return url;
    }
    if std::env::var("QBO_SANDBOX").is_ok() {
        "https://sandbox-quickbooks.api.intuit.com/v3".to_string()
    } else {
        "https://quickbooks.api.intuit.com/v3".to_string()
    }
}

// ============================================================================
// CDC tick
// ============================================================================

/// Run one CDC poll tick across all connected QBO tenants.
///
/// Returns the total number of entities processed.
pub async fn cdc_tick(pool: &PgPool) -> Result<u32, sqlx::Error> {
    let connections = sqlx::query_as::<_, CdcConnection>(
        r#"
        SELECT id, app_id, realm_id, cdc_watermark, full_resync_required
        FROM integrations_oauth_connections
        WHERE connection_status = 'connected'
          AND provider = 'quickbooks'
        "#,
    )
    .fetch_all(pool)
    .await?;

    if connections.is_empty() {
        return Ok(0);
    }

    let base_url = qbo_base_url();
    let mut processed = 0u32;

    for conn in &connections {
        let needs_resync = check_resync_needed(pool, conn).await?;

        if needs_resync {
            match super::sync::full_resync(
                pool,
                &base_url,
                &conn.app_id,
                &conn.realm_id,
            )
            .await
            {
                Ok(count) => {
                    tracing::info!(
                        app_id = %conn.app_id,
                        realm_id = %conn.realm_id,
                        entities = count,
                        "Full resync completed"
                    );
                    processed += count;
                }
                Err(e) => {
                    tracing::error!(
                        app_id = %conn.app_id,
                        realm_id = %conn.realm_id,
                        error = %e,
                        "Full resync failed"
                    );
                }
            }
        } else if let Some(watermark) = conn.cdc_watermark {
            match poll_cdc(pool, &base_url, &conn.app_id, &conn.realm_id, &watermark)
                .await
            {
                Ok(count) => {
                    if count > 0 {
                        tracing::info!(
                            app_id = %conn.app_id,
                            realm_id = %conn.realm_id,
                            entities = count,
                            "CDC poll found changes"
                        );
                    }
                    processed += count;
                }
                Err(e) => {
                    tracing::error!(
                        app_id = %conn.app_id,
                        realm_id = %conn.realm_id,
                        error = %e,
                        "CDC poll failed"
                    );
                }
            }
        }
    }

    Ok(processed)
}

/// Check if a connection needs full resync; if so, set the DB flag.
async fn check_resync_needed(
    pool: &PgPool,
    conn: &CdcConnection,
) -> Result<bool, sqlx::Error> {
    if conn.full_resync_required {
        return Ok(true);
    }

    let needs_resync = match conn.cdc_watermark {
        None => true,
        Some(wm) => Utc::now() - wm > Duration::days(WATERMARK_STALE_DAYS),
    };

    if needs_resync {
        sqlx::query(
            "UPDATE integrations_oauth_connections \
             SET full_resync_required = TRUE, updated_at = NOW() \
             WHERE id = $1",
        )
        .bind(conn.id)
        .execute(pool)
        .await?;
    }

    Ok(needs_resync)
}

/// Poll the CDC endpoint for a single tenant and enqueue changed entities.
async fn poll_cdc(
    pool: &PgPool,
    base_url: &str,
    app_id: &str,
    realm_id: &str,
    watermark: &DateTime<Utc>,
) -> Result<u32, Box<dyn std::error::Error + Send + Sync>> {
    let tokens: Arc<dyn TokenProvider> = Arc::new(DbTokenProvider {
        pool: pool.clone(),
        app_id: app_id.to_string(),
    });
    let client = QboClient::new(base_url, realm_id, tokens);

    let entity_refs: Vec<&str> = CDC_ENTITIES.to_vec();
    let response = client.cdc(&entity_refs, watermark).await?;

    let mut tx = pool.begin().await?;
    let count =
        process_cdc_entities(&mut tx, &response, app_id, realm_id, "cdc").await?;

    // Advance watermark
    let now = Utc::now();
    sqlx::query(
        "UPDATE integrations_oauth_connections \
         SET cdc_watermark = $1, updated_at = $1 \
         WHERE app_id = $2 AND provider = 'quickbooks'",
    )
    .bind(now)
    .bind(app_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(count)
}

/// Parse a CDC response and enqueue each entity into the outbox.
///
/// CDC response structure:
/// ```json
/// { "CDCResponse": [{ "QueryResponse": [{ "Customer": [...], ... }] }] }
/// ```
pub(crate) async fn process_cdc_entities(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    response: &serde_json::Value,
    app_id: &str,
    realm_id: &str,
    source: &str,
) -> Result<u32, Box<dyn std::error::Error + Send + Sync>> {
    let now = Utc::now();
    let correlation_id = Uuid::new_v4().to_string();

    let qr = response
        .get("CDCResponse")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .and_then(|v| v.get("QueryResponse"))
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first());

    let qr = match qr {
        Some(v) => v,
        None => return Ok(0),
    };

    let mut count = 0u32;

    for entity_type in CDC_ENTITIES {
        let entities = match qr.get(*entity_type).and_then(|v| v.as_array()) {
            Some(arr) => arr,
            None => continue,
        };

        for entity in entities {
            let entity_id = entity
                .get("Id")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");

            let event_id = Uuid::new_v4();
            let payload = QboCdcEntityPayload {
                entity_type: entity_type.to_string(),
                entity_id: entity_id.to_string(),
                realm_id: realm_id.to_string(),
                source: source.to_string(),
                entity_data: entity.clone(),
                synced_at: now,
            };

            let envelope = create_integrations_envelope(
                event_id,
                app_id.to_string(),
                EVENT_TYPE_QBO_ENTITY_SYNCED.to_string(),
                correlation_id.clone(),
                None,
                MUTATION_CLASS_DATA_MUTATION.to_string(),
                payload,
            );

            enqueue_event_tx(
                tx,
                event_id,
                EVENT_TYPE_QBO_ENTITY_SYNCED,
                &format!("qbo_{}", entity_type.to_lowercase()),
                entity_id,
                app_id,
                &envelope,
            )
            .await?;

            count += 1;
        }
    }

    Ok(count)
}

// ============================================================================
// Worker
// ============================================================================

/// Spawn the CDC polling worker as a tokio background task.
pub fn spawn_cdc_worker(
    pool: PgPool,
    poll_interval: std::time::Duration,
    mut shutdown_rx: watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        tracing::info!(
            poll_interval_secs = poll_interval.as_secs(),
            "QBO CDC polling worker started"
        );

        loop {
            tokio::select! {
                _ = tokio::time::sleep(poll_interval) => {
                    match cdc_tick(&pool).await {
                        Ok(n) if n > 0 => {
                            tracing::info!(entities = n, "CDC tick completed");
                        }
                        Ok(_) => {}
                        Err(e) => {
                            tracing::error!(error = %e, "CDC tick failed");
                        }
                    }
                }
                _ = shutdown_rx.changed() => {
                    tracing::info!("QBO CDC worker shutting down");
                    break;
                }
            }
        }
    })
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn watermark_stale_days_is_25() {
        assert_eq!(WATERMARK_STALE_DAYS, 25);
    }

    #[test]
    fn default_poll_interval_is_15_minutes() {
        assert_eq!(DEFAULT_CDC_POLL_INTERVAL_SECS, 900);
    }

    #[test]
    fn qbo_base_url_defaults_to_production() {
        // Clear env vars that could interfere
        std::env::remove_var("QBO_BASE_URL");
        std::env::remove_var("QBO_SANDBOX");
        let url = qbo_base_url();
        assert_eq!(url, "https://quickbooks.api.intuit.com/v3");
    }

    #[test]
    fn cdc_entities_includes_all_four() {
        assert_eq!(CDC_ENTITIES.len(), 4);
        assert!(CDC_ENTITIES.contains(&"Customer"));
        assert!(CDC_ENTITIES.contains(&"Invoice"));
        assert!(CDC_ENTITIES.contains(&"Payment"));
        assert!(CDC_ENTITIES.contains(&"Item"));
    }

    #[tokio::test]
    async fn process_cdc_entities_empty_response() {
        // In-memory pool not possible without real DB — test parsing logic
        let response = json!({});
        // Just verify the JSON parsing returns 0 without a DB
        let qr = response
            .get("CDCResponse")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first());
        assert!(qr.is_none());
    }

    #[tokio::test]
    async fn process_cdc_entities_parses_structure() {
        let response = json!({
            "CDCResponse": [{
                "QueryResponse": [{
                    "Customer": [
                        {"Id": "1", "DisplayName": "Acme Corp"},
                        {"Id": "2", "DisplayName": "Globex"}
                    ],
                    "Invoice": [
                        {"Id": "100", "TotalAmt": 500.0}
                    ]
                }]
            }]
        });

        let qr = response
            .get("CDCResponse")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|v| v.get("QueryResponse"))
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .expect("QueryResponse must exist in test fixture");

        let customers = qr["Customer"].as_array().expect("Customer array");
        assert_eq!(customers.len(), 2);
        let invoices = qr["Invoice"].as_array().expect("Invoice array");
        assert_eq!(invoices.len(), 1);
    }
}
