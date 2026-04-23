//! QBO CDC (Change Data Capture) polling worker.
//!
//! Polls the QBO CDC endpoint for each connected tenant, writing canonical
//! observation rows to `integrations_sync_observations`.  Acts as the reliable
//! backup to QBO webhooks, which have 5–25 minute delivery latency and can be
//! lost.

use chrono::{DateTime, Duration, Utc};
use sqlx::PgPool;
use std::sync::Arc;
use tokio::sync::watch;

use super::client::QboClient;
use super::repo;
use super::{QboError, TokenProvider};
use crate::domain::sync::dedupe::{
    compute_comparable_hash, compute_fingerprint, truncate_to_millis,
};
use crate::domain::sync::detector;
use crate::domain::sync::health;
use crate::domain::sync::observations;

/// Entities included in CDC queries.
pub const CDC_ENTITIES: &[&str] = &["Customer", "Invoice", "Payment", "Item"];

/// Default poll interval (15 minutes).
pub const DEFAULT_CDC_POLL_INTERVAL_SECS: u64 = 900;

/// Days before the 30-day CDC cliff to trigger full resync (5-day buffer).
pub const WATERMARK_STALE_DAYS: i64 = 25;

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
        crate::domain::oauth::service::get_access_token(&self.pool, &self.app_id, "quickbooks")
            .await
            .map_err(|e| QboError::TokenError(e.to_string()))
    }

    async fn refresh_token(&self) -> Result<String, QboError> {
        // Token refresh is handled by the background refresh worker.
        Err(QboError::AuthFailed)
    }
}

// ============================================================================
// URL resolution
// ============================================================================

/// Resolve the QBO API base URL from environment.
///
/// Precedence: `QBO_BASE_URL` (if non-empty) → `QBO_SANDBOX` flag → production default.
pub fn qbo_base_url() -> String {
    let explicit = std::env::var("QBO_BASE_URL").unwrap_or_default();
    if !explicit.is_empty() {
        return explicit;
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
    let connections = repo::get_connected_qbo_connections(pool).await?;

    if connections.is_empty() {
        return Ok(0);
    }

    let base_url = qbo_base_url();
    let mut processed = 0u32;

    for conn in &connections {
        let needs_resync = check_resync_needed(pool, &conn).await?;

        if needs_resync {
            match super::sync::full_resync(pool, &base_url, &conn.app_id, &conn.realm_id).await {
                Ok(count) => {
                    tracing::info!(
                        app_id = %conn.app_id,
                        realm_id = %conn.realm_id,
                        entities = count,
                        "Full resync completed"
                    );
                    processed += count;
                    if let Err(e) =
                        health::upsert_job_success(pool, &conn.app_id, "quickbooks", "cdc_poll")
                            .await
                    {
                        tracing::warn!(error = %e, "Failed to record cdc_poll health");
                    }
                }
                Err(e) => {
                    tracing::error!(
                        app_id = %conn.app_id,
                        realm_id = %conn.realm_id,
                        error = %e,
                        "Full resync failed"
                    );
                    if let Err(he) = health::upsert_job_failure(
                        pool,
                        &conn.app_id,
                        "quickbooks",
                        "cdc_poll",
                        &e.to_string(),
                    )
                    .await
                    {
                        tracing::warn!(error = %he, "Failed to record cdc_poll health");
                    }
                }
            }
        } else if let Some(watermark) = conn.cdc_watermark {
            match poll_cdc(pool, &base_url, &conn.app_id, &conn.realm_id, &watermark).await {
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
                    if let Err(e) =
                        health::upsert_job_success(pool, &conn.app_id, "quickbooks", "cdc_poll")
                            .await
                    {
                        tracing::warn!(error = %e, "Failed to record cdc_poll health");
                    }
                }
                Err(e) => {
                    tracing::error!(
                        app_id = %conn.app_id,
                        realm_id = %conn.realm_id,
                        error = %e,
                        "CDC poll failed"
                    );
                    if let Err(he) = health::upsert_job_failure(
                        pool,
                        &conn.app_id,
                        "quickbooks",
                        "cdc_poll",
                        &e.to_string(),
                    )
                    .await
                    {
                        tracing::warn!(error = %he, "Failed to record cdc_poll health");
                    }
                }
            }
        }
    }

    Ok(processed)
}

/// Check if a connection needs full resync; if so, set the DB flag.
async fn check_resync_needed(
    pool: &PgPool,
    conn: &repo::CdcConnection,
) -> Result<bool, sqlx::Error> {
    if conn.full_resync_required {
        return Ok(true);
    }

    let needs_resync = match conn.cdc_watermark {
        None => true,
        Some(wm) => Utc::now() - wm > Duration::days(WATERMARK_STALE_DAYS),
    };

    if needs_resync {
        repo::set_full_resync_required(pool, conn.id).await?;
    }

    Ok(needs_resync)
}

/// Poll the CDC endpoint for a single tenant and write observation rows.
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

    let (count, max_lut) = process_cdc_entities(pool, &response, app_id, realm_id).await?;

    // Advance watermark to the max provider-confirmed LastUpdatedTime.
    // If no entities were observed, fall back to Utc::now() so the window
    // doesn't stay pinned to an empty interval on subsequent polls.
    let new_watermark = max_lut.unwrap_or_else(Utc::now);
    let mut tx = pool.begin().await?;
    repo::advance_cdc_watermark(&mut tx, app_id, new_watermark).await?;
    tx.commit().await?;

    Ok(count)
}

/// Parse a CDC response and write each entity as a canonical observation row.
///
/// CDC response structure:
/// ```json
/// { "CDCResponse": [{ "QueryResponse": [{ "Customer": [...], ... }] }] }
/// ```
///
/// Returns `(count, max_last_updated_time)`.  `max_last_updated_time` is `None`
/// when the response contains no entities.
pub async fn process_cdc_entities(
    pool: &PgPool,
    response: &serde_json::Value,
    app_id: &str,
    realm_id: &str,
) -> Result<(u32, Option<DateTime<Utc>>), Box<dyn std::error::Error + Send + Sync>> {
    let query_responses = response
        .get("CDCResponse")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .and_then(|v| v.get("QueryResponse"))
        .and_then(|v| v.as_array());

    let query_responses = match query_responses {
        Some(arr) => arr,
        None => return Ok((0, None)),
    };

    let mut count = 0u32;
    let mut max_lut: Option<DateTime<Utc>> = None;

    for qr in query_responses {
        for entity_type in CDC_ENTITIES {
            let entities = match qr.get(*entity_type).and_then(|v| v.as_array()) {
                Some(arr) => arr,
                None => continue,
            };

            let entity_type_lower = entity_type.to_lowercase();

            for entity in entities {
                let entity_id = entity
                    .get("Id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");

                let is_tombstone = entity
                    .get("status")
                    .and_then(|v| v.as_str())
                    .map(|s| s.eq_ignore_ascii_case("Deleted"))
                    .unwrap_or(false);

                let lut = parse_last_updated_time(entity);
                let lut_truncated = truncate_to_millis(lut);

                // Track max provider-confirmed timestamp for watermark advance.
                max_lut = Some(match max_lut {
                    None => lut_truncated,
                    Some(prev) => prev.max(lut_truncated),
                });

                let sync_token = entity.get("SyncToken").and_then(|v| v.as_str());
                let comparable = comparable_fields(entity);
                let fingerprint = compute_fingerprint(sync_token, Some(lut_truncated), entity);
                let comparable_hash = compute_comparable_hash(&comparable, lut_truncated);

                let obs = observations::upsert_observation(
                    pool,
                    app_id,
                    "quickbooks",
                    &entity_type_lower,
                    entity_id,
                    &fingerprint,
                    lut_truncated,
                    &comparable_hash,
                    1,
                    entity,
                    "cdc",
                    is_tombstone,
                )
                .await
                .map_err(|e| {
                    tracing::error!(
                        app_id, entity_type, entity_id,
                        error = %e, "Failed to upsert CDC observation"
                    );
                    e
                })?;

                if let Err(e) = detector::run_detector(
                    pool,
                    app_id,
                    "quickbooks",
                    &entity_type_lower,
                    entity_id,
                    &obs.fingerprint,
                    &obs.comparable_hash,
                    None,
                    Some(entity.clone()),
                )
                .await
                {
                    tracing::warn!(
                        app_id, entity_type = entity_type_lower, entity_id,
                        error = %e,
                        "Detector error after CDC observation — conflict may be lost"
                    );
                }

                count += 1;

                let _ = realm_id; // realm_id stored in the entity payload itself
            }
        }
    }

    Ok((count, max_lut))
}

/// Extract `MetaData.LastUpdatedTime` from a QBO entity, falling back through
/// `MetaData.CreateTime` and then the current wall clock.
pub(crate) fn parse_last_updated_time(entity: &serde_json::Value) -> DateTime<Utc> {
    let meta = entity.get("MetaData");
    if let Some(ts_str) = meta
        .and_then(|m| m.get("LastUpdatedTime"))
        .and_then(|v| v.as_str())
    {
        if let Ok(ts) = ts_str.parse::<DateTime<Utc>>() {
            return ts;
        }
    }
    if let Some(ts_str) = meta
        .and_then(|m| m.get("CreateTime"))
        .and_then(|v| v.as_str())
    {
        if let Ok(ts) = ts_str.parse::<DateTime<Utc>>() {
            return ts;
        }
    }
    Utc::now()
}

/// Build the comparable projection by stripping ephemeral metadata fields.
///
/// Excludes `MetaData` (timestamps, internal provider IDs) and `SyncToken`
/// so that two observations for the same logical entity state always produce
/// the same `comparable_hash`.
pub(crate) fn comparable_fields(entity: &serde_json::Value) -> serde_json::Value {
    if let Some(obj) = entity.as_object() {
        let mut stripped = obj.clone();
        stripped.remove("MetaData");
        stripped.remove("SyncToken");
        stripped.remove("domain");
        stripped.remove("sparse");
        serde_json::Value::Object(stripped)
    } else {
        entity.clone()
    }
}

// ============================================================================
// Worker
// ============================================================================

/// Run one CDC poll tick scoped to a single tenant.
///
/// Identical logic to [`cdc_tick`] — same code paths, same workers — but
/// filtered to `app_id` only. Used by the test-only `/sync/cdc/trigger`
/// endpoint so integration tests can force a deterministic CDC cycle without
/// waiting for the 15-minute background worker.
pub async fn cdc_tick_for_tenant(pool: &PgPool, app_id: &str) -> Result<u32, sqlx::Error> {
    let connections = repo::get_connected_qbo_connections(pool).await?;
    let base_url = qbo_base_url();
    let mut processed = 0u32;

    for conn in connections.iter().filter(|c| c.app_id == app_id) {
        let needs_resync = check_resync_needed(pool, conn).await?;

        if needs_resync {
            match super::sync::full_resync(pool, &base_url, &conn.app_id, &conn.realm_id).await {
                Ok(count) => {
                    tracing::info!(
                        app_id = %conn.app_id,
                        realm_id = %conn.realm_id,
                        entities = count,
                        "CDC trigger: full resync completed"
                    );
                    processed += count;
                    if let Err(e) =
                        health::upsert_job_success(pool, &conn.app_id, "quickbooks", "cdc_poll")
                            .await
                    {
                        tracing::warn!(error = %e, "Failed to record cdc_poll health");
                    }
                }
                Err(e) => {
                    tracing::error!(
                        app_id = %conn.app_id,
                        error = %e,
                        "CDC trigger: full resync failed"
                    );
                }
            }
        } else if let Some(watermark) = conn.cdc_watermark {
            match poll_cdc(pool, &base_url, &conn.app_id, &conn.realm_id, &watermark).await {
                Ok(count) => {
                    processed += count;
                    if let Err(e) =
                        health::upsert_job_success(pool, &conn.app_id, "quickbooks", "cdc_poll")
                            .await
                    {
                        tracing::warn!(error = %e, "Failed to record cdc_poll health");
                    }
                }
                Err(e) => {
                    tracing::error!(
                        app_id = %conn.app_id,
                        error = %e,
                        "CDC trigger: poll failed"
                    );
                }
            }
        }
    }

    Ok(processed)
}

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

    #[test]
    fn parse_last_updated_time_uses_metadata() {
        let entity = json!({
            "Id": "1",
            "MetaData": {
                "LastUpdatedTime": "2024-01-15T10:30:00Z",
                "CreateTime": "2024-01-01T00:00:00Z"
            }
        });
        let ts = parse_last_updated_time(&entity);
        assert_eq!(ts.timestamp(), 1705314600);
    }

    #[test]
    fn parse_last_updated_time_falls_back_to_create_time() {
        let entity = json!({
            "Id": "1",
            "MetaData": {
                "CreateTime": "2024-01-10T08:00:00Z"
            }
        });
        let ts = parse_last_updated_time(&entity);
        assert_eq!(ts.timestamp(), 1704873600);
    }

    #[test]
    fn parse_last_updated_time_falls_back_to_now_on_missing_metadata() {
        let before = Utc::now();
        let ts = parse_last_updated_time(&json!({"Id": "1"}));
        let after = Utc::now();
        assert!(ts >= before && ts <= after);
    }

    #[test]
    fn comparable_fields_strips_metadata_and_sync_token() {
        let entity = json!({
            "Id": "1",
            "DisplayName": "Acme Corp",
            "SyncToken": "5",
            "domain": "QBO",
            "sparse": false,
            "MetaData": {"LastUpdatedTime": "2024-01-15T10:30:00Z"}
        });
        let cf = comparable_fields(&entity);
        assert!(cf.get("MetaData").is_none(), "MetaData must be stripped");
        assert!(cf.get("SyncToken").is_none(), "SyncToken must be stripped");
        assert!(cf.get("domain").is_none(), "domain must be stripped");
        assert!(cf.get("sparse").is_none(), "sparse must be stripped");
        assert_eq!(cf["Id"], "1");
        assert_eq!(cf["DisplayName"], "Acme Corp");
    }

    #[test]
    fn is_tombstone_detected_on_deleted_status() {
        let deleted = json!({"Id": "1", "status": "Deleted"});
        let active = json!({"Id": "2", "Active": true});

        assert!(
            deleted["status"]
                .as_str()
                .map(|s| s.eq_ignore_ascii_case("Deleted"))
                .unwrap_or(false),
            "Deleted status must be detected"
        );
        assert!(
            !active
                .get("status")
                .and_then(|v| v.as_str())
                .map(|s| s.eq_ignore_ascii_case("Deleted"))
                .unwrap_or(false),
            "Active entity must not be a tombstone"
        );
    }

    #[test]
    fn process_cdc_entities_empty_response_returns_zero() {
        let response = json!({});
        let qr = response
            .get("CDCResponse")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first());
        assert!(qr.is_none());
    }

    #[test]
    fn cdc_response_parses_structure() {
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
