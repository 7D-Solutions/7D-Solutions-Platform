//! eBay fulfillment write-back.
//!
//! ## Overview
//! Subscribes to `shipping_receiving.outbound_shipped`. When an outbound
//! shipment carries lines with `source_ref_type = "ebay_order"`, resolves the
//! eBay order ID from the referenced `integrations_file_jobs` row and pushes
//! the tracking number + carrier code to the eBay Fulfillment API.
//!
//! ## Shipment line → eBay order ID
//! `OutboundShippedLine.source_ref_id` must be the UUID of an
//! `integrations_file_jobs` row with `parser_type = "ebay_order"`.  The
//! `file_ref` column encodes `"ebay:order:{orderId}"` — the eBay order ID is
//! extracted from that prefix.
//!
//! ## Carrier code
//! Resolved via `integrations_external_refs`:
//! ```text
//! entity_type = "carrier_party"
//! entity_id   = {carrier_party_id}   (UUID as string)
//! system      = "ebay"
//! external_id = {ebay_carrier_code}  e.g. "USPS", "UPS", "FEDEX"
//! ```
//! Falls back to `"OTHER"` when no mapping is registered for the carrier.
//!
//! ## Idempotency
//! eBay returns 409 when the same tracking number already exists on an order.
//! This module treats 409 as success — pushing the same tracking twice is safe.

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use chrono::{DateTime, Utc};
use event_bus::{BusMessage, EventBus};
use futures::StreamExt;
use reqwest::Client;
use serde::Deserialize;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::connectors::repo as connector_repo;
use crate::domain::external_refs::repo as refs_repo;
use crate::domain::file_jobs::repo as file_job_repo;

// ── Constants ─────────────────────────────────────────────────────────────────

/// NATS subject for outbound shipped events (emitted by shipping-receiving).
pub const SUBJECT_OUTBOUND_SHIPPED: &str = "shipping_receiving.outbound_shipped";

const EBAY_TOKEN_URL_SANDBOX: &str =
    "https://api.sandbox.ebay.com/identity/v1/oauth2/token";
const EBAY_TOKEN_URL_PRODUCTION: &str =
    "https://api.ebay.com/identity/v1/oauth2/token";
const EBAY_FULFILLMENT_BASE_SANDBOX: &str =
    "https://api.sandbox.ebay.com/sell/fulfillment/v1/order";
const EBAY_FULFILLMENT_BASE_PRODUCTION: &str =
    "https://api.ebay.com/sell/fulfillment/v1/order";

/// OAuth2 scope required to create fulfillments (write access).
const SCOPE_SELL_FULFILLMENT: &str =
    "https://api.ebay.com/oauth/api_scope/sell.fulfillment";

/// Default eBay carrier code when no external-ref mapping exists.
const DEFAULT_CARRIER_CODE: &str = "OTHER";

/// Prefix used in file_ref for eBay order file jobs.
const EBAY_ORDER_FILE_REF_PREFIX: &str = "ebay:order:";

// ── Inbound event payload ─────────────────────────────────────────────────────

/// Mirrored from `shipping_receiving::events::contracts::OutboundShippedLine`.
///
/// Local copy to avoid a cross-crate dependency on the shipping-receiving lib.
#[derive(Debug, Clone, Deserialize)]
pub struct OutboundShippedLine {
    pub line_id: Uuid,
    pub sku: String,
    pub qty_shipped: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue_id: Option<Uuid>,
    /// Source document type, e.g. `"ebay_order"` or `"sales_order"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_ref_type: Option<String>,
    /// UUID of the source document (file_job ID when `source_ref_type = "ebay_order"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_ref_id: Option<Uuid>,
}

/// Mirrored from `shipping_receiving::events::contracts::OutboundShippedPayload`.
#[derive(Debug, Clone, Deserialize)]
pub struct OutboundShippedPayload {
    pub tenant_id: String,
    pub shipment_id: Uuid,
    pub lines: Vec<OutboundShippedLine>,
    pub shipped_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracking_number: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub carrier_party_id: Option<Uuid>,
}

/// Minimal deserialization wrapper for the event envelope.
#[derive(Debug, Deserialize)]
struct OutboundShippedEnvelope {
    payload: OutboundShippedPayload,
}

// ── OAuth2 token exchange ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct EbayTokenResponse {
    access_token: String,
}

/// Exchange client credentials for an eBay OAuth2 access token with fulfillment
/// write scope (`sell.fulfillment`).
async fn exchange_token(
    http_client: &Client,
    client_id: &str,
    client_secret: &str,
    token_url: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let credentials = BASE64.encode(format!("{}:{}", client_id, client_secret));

    let resp = http_client
        .post(token_url)
        .header("Authorization", format!("Basic {}", credentials))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .form(&[
            ("grant_type", "client_credentials"),
            ("scope", SCOPE_SELL_FULFILLMENT),
        ])
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(
            format!("eBay fulfillment token exchange failed ({}): {}", status, body).into(),
        );
    }

    let token: EbayTokenResponse = resp.json().await?;
    Ok(token.access_token)
}

// ── eBay Fulfillment API call ─────────────────────────────────────────────────

/// Push a tracking number + carrier code to eBay for a specific order.
///
/// Calls `POST {fulfillment_base_url}/{ebay_order_id}/shipping_fulfillment`.
///
/// eBay returns `204 No Content` on success.  A `409 Conflict` response means
/// the tracking number was already registered for this order — treated as
/// success (idempotent).
///
/// `fulfillment_base_url` is injected so tests can point at a local stub
/// instead of the real eBay API.
pub async fn push_tracking_to_ebay(
    http_client: &Client,
    access_token: &str,
    fulfillment_base_url: &str,
    ebay_order_id: &str,
    carrier_code: &str,
    tracking_number: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let url = format!("{}/{}/shipping_fulfillment", fulfillment_base_url, ebay_order_id);

    let body = serde_json::json!({
        "shippingCarrierCode": carrier_code,
        "trackingNumber":      tracking_number,
    });

    let resp = http_client
        .post(&url)
        .bearer_auth(access_token)
        .json(&body)
        .send()
        .await?;

    let status = resp.status();

    if status.is_success() {
        tracing::info!(
            ebay_order_id,
            carrier_code,
            tracking_number,
            "eBay fulfillment: tracking pushed successfully"
        );
        return Ok(());
    }

    // 409 = duplicate fulfillment — idempotent success
    if status.as_u16() == 409 {
        tracing::debug!(
            ebay_order_id,
            tracking_number,
            "eBay fulfillment: tracking already registered — idempotent skip"
        );
        return Ok(());
    }

    let body_text = resp.text().await.unwrap_or_default();
    Err(format!(
        "eBay Fulfillment API error ({}) for order {}: {}",
        status, ebay_order_id, body_text
    )
    .into())
}

// ── Core processing ───────────────────────────────────────────────────────────

/// Process a single `shipping_receiving.outbound_shipped` event.
///
/// Skips the event silently when:
/// - `tracking_number` is absent or empty, OR
/// - no lines carry `source_ref_type = "ebay_order"`, OR
/// - the tenant has no enabled eBay connector.
///
/// `fulfillment_base_url` and `token_url` are injected for testability.
/// Pass `None` to resolve them from the connector `environment` field.
pub async fn process_outbound_shipped(
    pool: &PgPool,
    http_client: &Client,
    payload: &OutboundShippedPayload,
    fulfillment_base_url: Option<&str>,
    token_url: Option<&str>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let tracking_number = match payload.tracking_number.as_deref().filter(|t| !t.is_empty()) {
        Some(t) => t,
        None => {
            tracing::debug!(
                shipment_id = %payload.shipment_id,
                "eBay fulfillment: no tracking number — skipping"
            );
            return Ok(());
        }
    };

    // Collect lines that reference eBay file_jobs.
    let ebay_lines: Vec<&OutboundShippedLine> = payload
        .lines
        .iter()
        .filter(|l| l.source_ref_type.as_deref() == Some("ebay_order"))
        .collect();

    if ebay_lines.is_empty() {
        tracing::debug!(
            shipment_id = %payload.shipment_id,
            "eBay fulfillment: no eBay-sourced lines — skipping"
        );
        return Ok(());
    }

    let tenant_id = payload.tenant_id.as_str();

    // Load eBay connector config for this tenant.
    let connector = match connector_repo::get_config_by_type(pool, tenant_id, "ebay").await? {
        Some(c) if c.enabled => c,
        _ => {
            tracing::warn!(
                tenant_id,
                shipment_id = %payload.shipment_id,
                "eBay fulfillment: no enabled eBay connector — skipping"
            );
            return Ok(());
        }
    };

    let client_id = connector
        .config
        .get("client_id")
        .and_then(|v| v.as_str())
        .ok_or("eBay fulfillment: missing client_id in connector config")?;
    let client_secret = connector
        .config
        .get("client_secret")
        .and_then(|v| v.as_str())
        .ok_or("eBay fulfillment: missing client_secret in connector config")?;
    let environment = connector
        .config
        .get("environment")
        .and_then(|v| v.as_str())
        .unwrap_or("SANDBOX");
    let is_sandbox = environment.eq_ignore_ascii_case("SANDBOX");

    let resolved_token_url = token_url.unwrap_or(if is_sandbox {
        EBAY_TOKEN_URL_SANDBOX
    } else {
        EBAY_TOKEN_URL_PRODUCTION
    });
    let resolved_fulfillment_url = fulfillment_base_url.unwrap_or(if is_sandbox {
        EBAY_FULFILLMENT_BASE_SANDBOX
    } else {
        EBAY_FULFILLMENT_BASE_PRODUCTION
    });

    // Exchange token once for all lines (write scope).
    let access_token =
        exchange_token(http_client, client_id, client_secret, resolved_token_url).await?;

    // Resolve carrier code once for the whole shipment.
    let carrier_code =
        resolve_carrier_code(pool, tenant_id, payload.carrier_party_id).await;

    // Process unique eBay orders (multiple lines may share the same order).
    let mut processed: std::collections::HashSet<String> = std::collections::HashSet::new();

    for line in ebay_lines {
        let file_job_id = match line.source_ref_id {
            Some(id) => id,
            None => {
                tracing::warn!(
                    tenant_id,
                    line_id = %line.line_id,
                    "eBay fulfillment: ebay_order line missing source_ref_id — skipping line"
                );
                continue;
            }
        };

        let job = match file_job_repo::get_by_id(pool, tenant_id, file_job_id).await? {
            Some(j) if j.parser_type == "ebay_order" => j,
            Some(j) => {
                tracing::warn!(
                    tenant_id,
                    file_job_id = %file_job_id,
                    parser_type = %j.parser_type,
                    "eBay fulfillment: file_job is not parser_type=ebay_order — skipping line"
                );
                continue;
            }
            None => {
                tracing::warn!(
                    tenant_id,
                    file_job_id = %file_job_id,
                    "eBay fulfillment: file_job not found — skipping line"
                );
                continue;
            }
        };

        let ebay_order_id = match job.file_ref.strip_prefix(EBAY_ORDER_FILE_REF_PREFIX) {
            Some(id) if !id.is_empty() => id.to_string(),
            _ => {
                tracing::warn!(
                    tenant_id,
                    file_ref = %job.file_ref,
                    "eBay fulfillment: unexpected file_ref format — skipping line"
                );
                continue;
            }
        };

        if processed.contains(&ebay_order_id) {
            continue; // Already pushed tracking for this order in this event.
        }
        processed.insert(ebay_order_id.clone());

        push_tracking_to_ebay(
            http_client,
            &access_token,
            resolved_fulfillment_url,
            &ebay_order_id,
            &carrier_code,
            tracking_number,
        )
        .await?;
    }

    Ok(())
}

/// Resolve the eBay carrier code via `integrations_external_refs`.
///
/// Looks for `entity_type = "carrier_party"`, `entity_id = {carrier_party_id}`,
/// `system = "ebay"`.  Falls back to `"OTHER"` on miss or error.
async fn resolve_carrier_code(
    pool: &PgPool,
    tenant_id: &str,
    carrier_party_id: Option<Uuid>,
) -> String {
    let Some(party_id) = carrier_party_id else {
        return DEFAULT_CARRIER_CODE.to_string();
    };

    match refs_repo::list_by_entity(pool, tenant_id, "carrier_party", &party_id.to_string())
        .await
    {
        Ok(refs) => refs
            .into_iter()
            .find(|r| r.system == "ebay")
            .map(|r| r.external_id)
            .unwrap_or_else(|| DEFAULT_CARRIER_CODE.to_string()),
        Err(e) => {
            tracing::warn!(
                tenant_id,
                carrier_party_id = %party_id,
                error = %e,
                "eBay fulfillment: carrier code lookup failed — using default"
            );
            DEFAULT_CARRIER_CODE.to_string()
        }
    }
}

// ── NATS consumer ─────────────────────────────────────────────────────────────

/// Start the eBay fulfillment consumer as a background task.
///
/// Subscribes to `shipping_receiving.outbound_shipped`.  On each message,
/// processes eBay-sourced lines and pushes tracking numbers to eBay.
pub fn start_ebay_fulfillment_consumer(bus: Arc<dyn EventBus>, pool: PgPool) {
    tokio::spawn(async move {
        let http_client = Client::new();
        tracing::info!("Integrations: starting eBay fulfillment consumer");

        let mut stream = match bus.subscribe(SUBJECT_OUTBOUND_SHIPPED).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(
                    error = %e,
                    subject = SUBJECT_OUTBOUND_SHIPPED,
                    "Integrations: failed to subscribe to outbound_shipped"
                );
                return;
            }
        };

        tracing::info!(
            subject = SUBJECT_OUTBOUND_SHIPPED,
            "Integrations: eBay fulfillment consumer subscribed"
        );

        while let Some(msg) = stream.next().await {
            if let Err(e) = handle_message(&pool, &http_client, &msg).await {
                tracing::error!(error = %e, "Integrations: eBay fulfillment processing failed");
            }
        }

        tracing::warn!("Integrations: eBay fulfillment consumer stopped");
    });
}

async fn handle_message(
    pool: &PgPool,
    http_client: &Client,
    msg: &BusMessage,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let envelope: OutboundShippedEnvelope = serde_json::from_slice(&msg.payload).map_err(|e| {
        format!("eBay fulfillment: failed to parse outbound_shipped event: {e}")
    })?;

    process_outbound_shipped(pool, http_client, &envelope.payload, None, None).await
}
