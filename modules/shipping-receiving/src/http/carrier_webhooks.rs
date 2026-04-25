//! Per-carrier inbound webhook handlers.
//!
//! Trust boundary: every handler MUST verify the carrier's auth before processing.
//! A missing or invalid signature always returns 401 — no partial processing.
//!
//! Invariant: these handlers update tracking_events and carrier_status on
//! shipments. They NEVER advance the shipment state machine (draft → confirmed
//! → in_transit etc.). That requires a dock-scan or manual-receipt API call.
//!
//! Carrier auth summary:
//!   UPS    — HMAC-SHA256, secret from env UPS_WEBHOOK_SECRET
//!   FedEx  — challenge-response on setup + HMAC-SHA256 for events, env FEDEX_WEBHOOK_SECRET
//!   USPS   — no webhook push (legacy API); this endpoint documents the policy
//!   R&L    — shared token in X-RL-Webhook-Token header, env RL_WEBHOOK_SECRET
//!   XPO    — HMAC-SHA256, env XPO_WEBHOOK_SECRET
//!   ODFL   — no webhook push; handled by odfl_poller background task
//!   Saia   — HMAC-SHA256, env SAIA_WEBHOOK_SECRET

use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use chrono::{DateTime, TimeZone, Utc};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::tracking::{self, sha256_hex};
use crate::events;
use crate::outbox;
use crate::AppState;

type HmacSha256 = Hmac<Sha256>;

// ── HMAC helpers ──────────────────────────────────────────────

/// Constant-time HMAC-SHA256 verification. Returns true on match.
fn verify_hmac_sha256(secret: &[u8], body: &[u8], expected_hex: &str) -> bool {
    let Ok(expected_bytes) = hex::decode(expected_hex) else {
        return false;
    };
    let mut mac = match HmacSha256::new_from_slice(secret) {
        Ok(m) => m,
        Err(_) => return false,
    };
    mac.update(body);
    mac.verify_slice(&expected_bytes).is_ok()
}

/// Read a webhook secret from the environment. Returns None (→ 401) if unset.
fn webhook_secret(env_key: &str) -> Option<String> {
    std::env::var(env_key).ok().filter(|s| !s.is_empty())
}

// ── Shared webhook pipeline ───────────────────────────────────

struct WebhookEvent<'a> {
    tracking_number: &'a str,
    carrier_code: &'a str,
    status: &'a str,
    status_dttm: DateTime<Utc>,
    location: Option<&'a str>,
}

/// Shared tail of the webhook pipeline: persist event, update carrier_status,
/// recompute master if this is a child shipment, emit outbox event.
async fn process_webhook_event(
    state: &AppState,
    event: WebhookEvent<'_>,
    raw_payload_hash: &str,
) -> Result<(), sqlx::Error> {
    let lookup =
        tracking::find_shipment_by_tracking(&state.pool, event.tracking_number).await?;

    let (shipment_id, tenant_id, parent_id) = match lookup {
        Some(row) => row,
        None => {
            // Tracking number not yet known — record the event without a shipment link
            tracking::record_tracking_event(
                &state.pool,
                "unknown",
                None,
                event.tracking_number,
                event.carrier_code,
                event.status,
                event.status_dttm,
                event.location,
                raw_payload_hash,
            )
            .await?;
            return Ok(());
        }
    };

    let inserted = tracking::record_tracking_event(
        &state.pool,
        &tenant_id,
        Some(shipment_id),
        event.tracking_number,
        event.carrier_code,
        event.status,
        event.status_dttm,
        event.location,
        raw_payload_hash,
    )
    .await?;

    // Idempotent replay: nothing new inserted → skip downstream effects
    if inserted.is_none() {
        return Ok(());
    }

    tracking::update_shipment_carrier_status(&state.pool, shipment_id, event.status).await?;

    if let Some(master_id) = parent_id {
        tracking::recompute_master_status(&state.pool, master_id).await?;
    }

    // Emit tracking.event_received outbox event (best-effort; non-fatal)
    let outbox_payload = serde_json::json!({
        "shipment_id": shipment_id,
        "tenant_id": tenant_id,
        "tracking_number": event.tracking_number,
        "carrier_code": event.carrier_code,
        "status": event.status,
        "status_dttm": event.status_dttm,
    });

    // Deterministic event_id from tracking_number + hash so replay is safe
    let event_id = Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("tracking:{}:{}", event.tracking_number, raw_payload_hash).as_bytes(),
    );

    let _ = outbox::enqueue_event_tx_pool(
        &state.pool,
        event_id,
        events::EVENT_TYPE_TRACKING_EVENT_RECEIVED,
        "shipment",
        &shipment_id.to_string(),
        &tenant_id,
        &outbox_payload,
    )
    .await;

    Ok(())
}

// ── UPS webhook ───────────────────────────────────────────────
//
// Auth: HMAC-SHA256, secret from env UPS_WEBHOOK_SECRET.
// Signature header: X-Ups-Webhook-Signature (format: "v1=<hex>")
//
// Payload:
// {
//   "type": "subscription_events",
//   "events": [{
//     "type": "TRACK",
//     "trackingNumber": "1Z...",
//     "localActivityDate": "20240315",
//     "localActivityTime": "143000",
//     "currentStatus": "I",
//     "location": "CITY, STATE, COUNTRY"
//   }]
// }

#[derive(Deserialize)]
struct UpsWebhookEvent {
    #[serde(rename = "trackingNumber")]
    tracking_number: String,
    #[serde(rename = "currentStatus")]
    current_status: String,
    #[serde(rename = "localActivityDate")]
    local_activity_date: Option<String>,
    #[serde(rename = "localActivityTime")]
    local_activity_time: Option<String>,
    location: Option<String>,
}

#[derive(Deserialize)]
struct UpsWebhookPayload {
    events: Vec<UpsWebhookEvent>,
}

pub async fn ups_webhook(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let secret = match webhook_secret("UPS_WEBHOOK_SECRET") {
        Some(s) => s,
        None => {
            tracing::error!("UPS_WEBHOOK_SECRET not configured");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // UPS sends "v1=<hex>" — extract the hex part
    let sig_header = headers
        .get("x-ups-webhook-signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let sig_hex = sig_header.trim_start_matches("v1=");

    if !verify_hmac_sha256(secret.as_bytes(), &body, sig_hex) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let raw_hash = sha256_hex(&body);

    let payload: UpsWebhookPayload = match serde_json::from_slice(&body) {
        Ok(p) => p,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };

    for ev in &payload.events {
        let status = ups_status_to_canonical(&ev.current_status);
        let status_dttm = parse_ups_datetime(
            ev.local_activity_date.as_deref(),
            ev.local_activity_time.as_deref(),
        );

        if let Err(e) = process_webhook_event(
            &state,
            WebhookEvent {
                tracking_number: &ev.tracking_number,
                carrier_code: "ups",
                status,
                status_dttm,
                location: ev.location.as_deref(),
            },
            &raw_hash,
        )
        .await
        {
            tracing::error!(error = %e, tracking_number = %ev.tracking_number, "SR: UPS webhook: DB error");
        }
    }

    StatusCode::OK.into_response()
}

fn ups_status_to_canonical(code: &str) -> &'static str {
    match code.to_ascii_uppercase().as_str() {
        "D" => tracking::STATUS_DELIVERED,
        "O" => tracking::STATUS_OUT_FOR_DELIVERY,
        "I" => tracking::STATUS_IN_TRANSIT,
        "P" | "OR" => tracking::STATUS_PICKED_UP,
        "M" => tracking::STATUS_PENDING,
        "RS" => tracking::STATUS_RETURNED,
        "X" | "NA" => tracking::STATUS_EXCEPTION,
        _ => tracking::STATUS_IN_TRANSIT,
    }
}

fn parse_ups_datetime(date: Option<&str>, time: Option<&str>) -> DateTime<Utc> {
    // date = "YYYYMMDD", time = "HHMMSS"
    if let (Some(d), Some(t)) = (date, time) {
        if d.len() == 8 && t.len() == 6 {
            let s = format!("{}-{}-{}T{}:{}:{}Z", &d[0..4], &d[4..6], &d[6..8], &t[0..2], &t[2..4], &t[4..6]);
            if let Ok(dt) = s.parse::<DateTime<Utc>>() {
                return dt;
            }
        }
    }
    Utc::now()
}

// ── FedEx webhook ─────────────────────────────────────────────
//
// Auth: HMAC-SHA256, env FEDEX_WEBHOOK_SECRET.
// Initial setup: FedEx posts {"event":{"eventType":"webhookSetup"},"challengeToken":"..."}
//   → handler must echo back {"challengeToken": "..."}.
// Regular events: normal payload with X-FedEx-Signature header.
//
// Payload (regular):
// {
//   "event": {
//     "eventType": "trackingUpdated",
//     "trackingInfo": {
//       "trackingNumber": "123456789012",
//       "latestStatusDetail": {
//         "code": "DL",
//         "derivedCode": "DL",
//         "statusByLocale": "Delivered"
//       },
//       "statusETA": null,
//       "lastUpdateTime": "2024-03-15T14:30:00Z"
//     }
//   },
//   "notificationEventTime": "2024-03-15T14:30:00Z"
// }

#[derive(Deserialize)]
struct FedExChallengePayload {
    #[serde(rename = "challengeToken")]
    challenge_token: Option<String>,
    event: Option<FedExEvent>,
}

#[derive(Deserialize)]
struct FedExEvent {
    #[serde(rename = "eventType")]
    event_type: String,
    #[serde(rename = "trackingInfo")]
    tracking_info: Option<FedExTrackingInfo>,
}

#[derive(Deserialize)]
struct FedExTrackingInfo {
    #[serde(rename = "trackingNumber")]
    tracking_number: String,
    #[serde(rename = "latestStatusDetail")]
    latest_status: Option<FedExStatusDetail>,
    #[serde(rename = "lastUpdateTime")]
    last_update_time: Option<String>,
}

#[derive(Deserialize)]
struct FedExStatusDetail {
    #[serde(rename = "derivedCode")]
    derived_code: Option<String>,
    #[serde(rename = "statusByLocale")]
    status_by_locale: Option<String>,
}

#[derive(Serialize)]
struct FedExChallengeResponse {
    #[serde(rename = "challengeToken")]
    challenge_token: String,
}

pub async fn fedex_webhook(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let secret = match webhook_secret("FEDEX_WEBHOOK_SECRET") {
        Some(s) => s,
        None => {
            tracing::error!("FEDEX_WEBHOOK_SECRET not configured");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let payload: FedExChallengePayload = match serde_json::from_slice(&body) {
        Ok(p) => p,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };

    // Challenge-response for initial webhook registration
    if let Some(token) = &payload.challenge_token {
        if payload.event.as_ref().map(|e| e.event_type.as_str()) == Some("webhookSetup") {
            return Json(FedExChallengeResponse {
                challenge_token: token.clone(),
            })
            .into_response();
        }
    }

    // Signature verification for real events
    let sig_hex = headers
        .get("x-fedex-signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !verify_hmac_sha256(secret.as_bytes(), &body, sig_hex) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let raw_hash = sha256_hex(&body);

    let Some(event) = payload.event else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let Some(info) = event.tracking_info else {
        return StatusCode::OK.into_response(); // non-tracking event type — ack
    };

    let status_code = info
        .latest_status
        .as_ref()
        .and_then(|s| s.derived_code.as_deref())
        .unwrap_or("");
    let status_locale = info
        .latest_status
        .as_ref()
        .and_then(|s| s.status_by_locale.as_deref())
        .unwrap_or("");
    let canonical_status = fedex_status_to_canonical(status_code, status_locale);

    let status_dttm = info
        .last_update_time
        .as_deref()
        .and_then(|s| s.parse::<DateTime<Utc>>().ok())
        .unwrap_or_else(Utc::now);

    if let Err(e) = process_webhook_event(
        &state,
        WebhookEvent {
            tracking_number: &info.tracking_number,
            carrier_code: "fedex",
            status: canonical_status,
            status_dttm,
            location: None,
        },
        &raw_hash,
    )
    .await
    {
        tracing::error!(error = %e, tracking_number = %info.tracking_number, "SR: FedEx webhook: DB error");
    }

    StatusCode::OK.into_response()
}

fn fedex_status_to_canonical(code: &str, locale: &str) -> &'static str {
    let locale_upper = locale.to_ascii_uppercase();
    match code {
        "DL" => tracking::STATUS_DELIVERED,
        "OD" => tracking::STATUS_OUT_FOR_DELIVERY,
        "IT" | "AR" => tracking::STATUS_IN_TRANSIT,
        "PU" | "PX" => tracking::STATUS_PICKED_UP,
        "LO" => tracking::STATUS_PENDING,
        "RS" => tracking::STATUS_RETURNED,
        "DE" | "SE" => tracking::STATUS_EXCEPTION,
        _ => {
            // Fallback: parse locale string
            if locale_upper.contains("DELIVER") {
                tracking::STATUS_DELIVERED
            } else if locale_upper.contains("OUT FOR") {
                tracking::STATUS_OUT_FOR_DELIVERY
            } else if locale_upper.contains("TRANSIT") || locale_upper.contains("ARRIVED") {
                tracking::STATUS_IN_TRANSIT
            } else if locale_upper.contains("PICKED") || locale_upper.contains("PICKUP") {
                tracking::STATUS_PICKED_UP
            } else if locale_upper.contains("EXCEPTION") {
                tracking::STATUS_EXCEPTION
            } else {
                tracking::STATUS_IN_TRANSIT
            }
        }
    }
}

// ── USPS webhook ──────────────────────────────────────────────
//
// USPS legacy Web Tools API does not support webhook push. This endpoint
// exists to document the policy and return 501 so carrier portal setup
// fails fast rather than silently dropping events.

pub async fn usps_webhook() -> impl IntoResponse {
    (
        StatusCode::NOT_IMPLEMENTED,
        "USPS does not support webhook push via the legacy Web Tools API. \
         Tracking is polled daily by the platform.",
    )
        .into_response()
}

// ── R&L webhook ───────────────────────────────────────────────
//
// Auth: shared token in X-RL-Webhook-Token header. env RL_WEBHOOK_SECRET.
//
// Payload:
// {
//   "proNumber": "123456789",
//   "status": "DEL",
//   "statusDescription": "Delivered",
//   "statusDate": "2024-03-15",
//   "statusTime": "14:30:00",
//   "location": "CITY, STATE"
// }

#[derive(Deserialize)]
struct RlWebhookPayload {
    #[serde(rename = "proNumber")]
    pro_number: String,
    status: String,
    #[serde(rename = "statusDate")]
    status_date: Option<String>,
    #[serde(rename = "statusTime")]
    status_time: Option<String>,
    location: Option<String>,
}

pub async fn rl_webhook(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let secret = match webhook_secret("RL_WEBHOOK_SECRET") {
        Some(s) => s,
        None => {
            tracing::error!("RL_WEBHOOK_SECRET not configured");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let token = headers
        .get("x-rl-webhook-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if token != secret {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let raw_hash = sha256_hex(&body);

    let payload: RlWebhookPayload = match serde_json::from_slice(&body) {
        Ok(p) => p,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };

    let canonical_status = rl_status_to_canonical(&payload.status);
    let status_dttm = parse_iso_datetime(
        payload.status_date.as_deref(),
        payload.status_time.as_deref(),
    );

    if let Err(e) = process_webhook_event(
        &state,
        WebhookEvent {
            tracking_number: &payload.pro_number,
            carrier_code: "rl",
            status: canonical_status,
            status_dttm,
            location: payload.location.as_deref(),
        },
        &raw_hash,
    )
    .await
    {
        tracing::error!(error = %e, pro_number = %payload.pro_number, "SR: R&L webhook: DB error");
    }

    StatusCode::OK.into_response()
}

fn rl_status_to_canonical(code: &str) -> &'static str {
    match code.to_ascii_uppercase().as_str() {
        "DEL" => tracking::STATUS_DELIVERED,
        "OUT" | "OFD" => tracking::STATUS_OUT_FOR_DELIVERY,
        "INT" | "ARR" => tracking::STATUS_IN_TRANSIT,
        "PCK" | "PUP" => tracking::STATUS_PICKED_UP,
        "EXC" | "DMG" => tracking::STATUS_EXCEPTION,
        "RET" => tracking::STATUS_RETURNED,
        _ => tracking::STATUS_IN_TRANSIT,
    }
}

// ── XPO webhook ───────────────────────────────────────────────
//
// Auth: HMAC-SHA256, env XPO_WEBHOOK_SECRET.
// Signature header: X-Xpo-Signature
//
// Payload:
// {
//   "proNumber": "123456789",
//   "eventType": "STATUS_UPDATE",
//   "status": "IN_TRANSIT",
//   "eventDateTime": "2024-03-15T14:30:00Z",
//   "location": "CITY, STATE"
// }

#[derive(Deserialize)]
struct XpoWebhookPayload {
    #[serde(rename = "proNumber")]
    pro_number: String,
    status: String,
    #[serde(rename = "eventDateTime")]
    event_date_time: Option<String>,
    location: Option<String>,
}

pub async fn xpo_webhook(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let secret = match webhook_secret("XPO_WEBHOOK_SECRET") {
        Some(s) => s,
        None => {
            tracing::error!("XPO_WEBHOOK_SECRET not configured");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let sig_hex = headers
        .get("x-xpo-signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !verify_hmac_sha256(secret.as_bytes(), &body, sig_hex) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let raw_hash = sha256_hex(&body);

    let payload: XpoWebhookPayload = match serde_json::from_slice(&body) {
        Ok(p) => p,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };

    let canonical_status = xpo_status_to_canonical(&payload.status);
    let status_dttm = payload
        .event_date_time
        .as_deref()
        .and_then(|s| s.parse::<DateTime<Utc>>().ok())
        .unwrap_or_else(Utc::now);

    if let Err(e) = process_webhook_event(
        &state,
        WebhookEvent {
            tracking_number: &payload.pro_number,
            carrier_code: "xpo",
            status: canonical_status,
            status_dttm,
            location: payload.location.as_deref(),
        },
        &raw_hash,
    )
    .await
    {
        tracing::error!(error = %e, pro_number = %payload.pro_number, "SR: XPO webhook: DB error");
    }

    StatusCode::OK.into_response()
}

fn xpo_status_to_canonical(code: &str) -> &'static str {
    match code.to_ascii_uppercase().as_str() {
        "DELIVERED" | "DEL" => tracking::STATUS_DELIVERED,
        "OUT_FOR_DELIVERY" | "OFD" => tracking::STATUS_OUT_FOR_DELIVERY,
        "IN_TRANSIT" | "ARR" | "ARRIVED" => tracking::STATUS_IN_TRANSIT,
        "PICKED_UP" | "PUP" => tracking::STATUS_PICKED_UP,
        "EXCEPTION" | "DMG" => tracking::STATUS_EXCEPTION,
        "RETURNED" | "RET" => tracking::STATUS_RETURNED,
        _ => tracking::STATUS_IN_TRANSIT,
    }
}

// ── ODFL webhook (no-op) ──────────────────────────────────────
//
// ODFL does not offer webhook push. Tracking is handled by the odfl_poller
// background task (15-minute interval). This endpoint returns 501 so portal
// setup fails fast.

pub async fn odfl_webhook() -> impl IntoResponse {
    (
        StatusCode::NOT_IMPLEMENTED,
        "ODFL does not support webhook push. Tracking is polled every 15 minutes.",
    )
        .into_response()
}

// ── Saia webhook ──────────────────────────────────────────────
//
// Auth: HMAC-SHA256, env SAIA_WEBHOOK_SECRET.
// Signature header: X-Saia-Signature
//
// Payload:
// {
//   "proNumber": "123456789",
//   "statusCode": "DEL",
//   "statusDescription": "Delivered",
//   "eventDate": "2024-03-15",
//   "eventTime": "14:30",
//   "location": "CITY, STATE"
// }

#[derive(Deserialize)]
struct SaiaWebhookPayload {
    #[serde(rename = "proNumber")]
    pro_number: String,
    #[serde(rename = "statusCode")]
    status_code: String,
    #[serde(rename = "eventDate")]
    event_date: Option<String>,
    #[serde(rename = "eventTime")]
    event_time: Option<String>,
    location: Option<String>,
}

pub async fn saia_webhook(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let secret = match webhook_secret("SAIA_WEBHOOK_SECRET") {
        Some(s) => s,
        None => {
            tracing::error!("SAIA_WEBHOOK_SECRET not configured");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let sig_hex = headers
        .get("x-saia-signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !verify_hmac_sha256(secret.as_bytes(), &body, sig_hex) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let raw_hash = sha256_hex(&body);

    let payload: SaiaWebhookPayload = match serde_json::from_slice(&body) {
        Ok(p) => p,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };

    let canonical_status = saia_status_to_canonical(&payload.status_code);
    let status_dttm = parse_iso_datetime(
        payload.event_date.as_deref(),
        payload.event_time.as_deref(),
    );

    if let Err(e) = process_webhook_event(
        &state,
        WebhookEvent {
            tracking_number: &payload.pro_number,
            carrier_code: "saia",
            status: canonical_status,
            status_dttm,
            location: payload.location.as_deref(),
        },
        &raw_hash,
    )
    .await
    {
        tracing::error!(error = %e, pro_number = %payload.pro_number, "SR: Saia webhook: DB error");
    }

    StatusCode::OK.into_response()
}

fn saia_status_to_canonical(code: &str) -> &'static str {
    match code.to_ascii_uppercase().as_str() {
        "DEL" => tracking::STATUS_DELIVERED,
        "OFD" => tracking::STATUS_OUT_FOR_DELIVERY,
        "INT" | "ARR" => tracking::STATUS_IN_TRANSIT,
        "PUP" | "PCK" => tracking::STATUS_PICKED_UP,
        "EXC" | "DMG" => tracking::STATUS_EXCEPTION,
        "RET" => tracking::STATUS_RETURNED,
        _ => tracking::STATUS_IN_TRANSIT,
    }
}

// ── Shared datetime parsing ───────────────────────────────────

/// Parse "YYYY-MM-DD" + optional "HH:MM[:SS]" into UTC DateTime.
fn parse_iso_datetime(date: Option<&str>, time: Option<&str>) -> DateTime<Utc> {
    if let Some(d) = date {
        let dt_str = if let Some(t) = time {
            format!("{}T{}:00Z", d, t.trim_end_matches(":00").trim())
        } else {
            format!("{}T00:00:00Z", d)
        };
        if let Ok(dt) = dt_str.parse::<DateTime<Utc>>() {
            return dt;
        }
    }
    Utc::now()
}

// ── Unit tests ────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_rejects_wrong_signature() {
        assert!(!verify_hmac_sha256(b"secret", b"body", "0000000000000000"));
    }

    #[test]
    fn hmac_accepts_correct_signature() -> Result<(), Box<dyn std::error::Error>> {
        // Pre-computed: HMAC-SHA256("secret", "body")
        let mut mac = HmacSha256::new_from_slice(b"secret")?;
        mac.update(b"body");
        let hex = hex::encode(mac.finalize().into_bytes());
        assert!(verify_hmac_sha256(b"secret", b"body", &hex));
        Ok(())
    }

    #[test]
    fn ups_status_mapping() {
        assert_eq!(ups_status_to_canonical("D"), tracking::STATUS_DELIVERED);
        assert_eq!(ups_status_to_canonical("I"), tracking::STATUS_IN_TRANSIT);
        assert_eq!(ups_status_to_canonical("O"), tracking::STATUS_OUT_FOR_DELIVERY);
        assert_eq!(ups_status_to_canonical("X"), tracking::STATUS_EXCEPTION);
    }

    #[test]
    fn fedex_status_mapping() {
        assert_eq!(fedex_status_to_canonical("DL", ""), tracking::STATUS_DELIVERED);
        assert_eq!(fedex_status_to_canonical("DE", ""), tracking::STATUS_EXCEPTION);
        assert_eq!(fedex_status_to_canonical("IT", ""), tracking::STATUS_IN_TRANSIT);
    }

    #[test]
    fn rl_status_mapping() {
        assert_eq!(rl_status_to_canonical("DEL"), tracking::STATUS_DELIVERED);
        assert_eq!(rl_status_to_canonical("INT"), tracking::STATUS_IN_TRANSIT);
        assert_eq!(rl_status_to_canonical("EXC"), tracking::STATUS_EXCEPTION);
    }

    #[test]
    fn saia_status_mapping() {
        assert_eq!(saia_status_to_canonical("DEL"), tracking::STATUS_DELIVERED);
        assert_eq!(saia_status_to_canonical("OFD"), tracking::STATUS_OUT_FOR_DELIVERY);
        assert_eq!(saia_status_to_canonical("PUP"), tracking::STATUS_PICKED_UP);
    }

    #[test]
    fn xpo_status_mapping() {
        assert_eq!(xpo_status_to_canonical("DELIVERED"), tracking::STATUS_DELIVERED);
        assert_eq!(xpo_status_to_canonical("IN_TRANSIT"), tracking::STATUS_IN_TRANSIT);
        assert_eq!(xpo_status_to_canonical("EXCEPTION"), tracking::STATUS_EXCEPTION);
    }
}
