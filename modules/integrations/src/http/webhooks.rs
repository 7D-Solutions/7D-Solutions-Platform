//! HTTP handlers for inbound webhook ingestion.
//!
//! ## Route
//! `POST /api/webhooks/inbound/{system}`
//!
//! Accepts a raw JSON body from an external system, verifies the signature,
//! persists the payload, and routes a domain event via the outbox.
//!
//! ## Idempotency
//! If the source system supplies a dedup key (e.g. Stripe event ID), it
//! should be passed as the `X-Webhook-Id` header. Duplicate delivery
//! returns `200 OK` without re-processing.
//!
//! ## Supported Systems
//! - `stripe` — HMAC-SHA256 via `Stripe-Signature` header.
//! - `github` — HMAC-SHA256 via `X-Hub-Signature-256` header.
//! - `internal` — No signature required.

use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::Json,
    Extension,
};
use security::VerifiedClaims;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::domain::webhooks::{IngestWebhookRequest, WebhookError, WebhookService};
use crate::AppState;

/// `POST /api/webhooks/inbound/{system}`
pub async fn inbound_webhook(
    State(state): State<Arc<AppState>>,
    Path(system): Path<String>,
    claims: Option<Extension<VerifiedClaims>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    // ── Extract app_id from JWT VerifiedClaims ────────────────────────────
    let app_id = match &claims {
        Some(Extension(c)) => c.tenant_id.to_string(),
        None => {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "Missing or invalid authentication" })),
            ));
        }
    };

    // ── Convert headers to HashMap<String, String> (lowercase) ────────────
    let header_map: std::collections::HashMap<String, String> = headers
        .iter()
        .filter_map(|(k, v)| {
            v.to_str().ok().map(|val| (k.as_str().to_lowercase(), val.to_string()))
        })
        .collect();

    // ── Parse raw body as JSON ─────────────────────────────────────────────
    let raw_payload: Value = serde_json::from_slice(&body).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": format!("Invalid JSON body: {}", e) })),
        )
    })?;

    // ── Extract idempotency key and event_type from headers / payload ──────
    let idempotency_key = header_map
        .get("x-webhook-id")
        .cloned()
        .or_else(|| extract_stripe_event_id(&raw_payload, &system));

    let event_type = extract_event_type(&raw_payload, &system);

    let req = IngestWebhookRequest {
        app_id,
        system: system.clone(),
        event_type,
        idempotency_key,
        raw_payload,
        headers: header_map,
    };

    let svc = WebhookService::new(state.pool.clone());
    match svc.ingest(req, &body).await {
        Ok(result) => {
            let status = if result.is_duplicate { "duplicate" } else { "accepted" };
            Ok(Json(json!({
                "status": status,
                "ingest_id": result.ingest_id,
            })))
        }
        Err(WebhookError::SignatureVerification(msg)) => Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": format!("Signature verification failed: {}", msg) })),
        )),
        Err(WebhookError::UnsupportedSystem { system }) => Err((
            StatusCode::NOT_FOUND,
            Json(json!({ "error": format!("Unknown webhook system: {}", system) })),
        )),
        Err(WebhookError::MalformedPayload(msg)) => Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": msg })),
        )),
        Err(e) => {
            tracing::error!(system = %system, error = %e, "Webhook ingest error");
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "Internal error" })),
            ))
        }
    }
}

/// Extract Stripe event ID from the payload as idempotency key.
fn extract_stripe_event_id(payload: &Value, system: &str) -> Option<String> {
    if system == "stripe" {
        payload.get("id")?.as_str().map(str::to_string)
    } else {
        None
    }
}

/// Extract event type from the payload body based on source system.
fn extract_event_type(payload: &Value, system: &str) -> Option<String> {
    match system {
        "stripe" => payload.get("type")?.as_str().map(str::to_string),
        "github" => None, // GitHub uses X-GitHub-Event header — captured in headers map
        _ => payload.get("event_type")
            .or_else(|| payload.get("type"))
            .and_then(|v| v.as_str())
            .map(str::to_string),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_extract_stripe_event_id() {
        let payload = json!({ "id": "evt_123", "type": "payment_intent.succeeded" });
        assert_eq!(
            extract_stripe_event_id(&payload, "stripe"),
            Some("evt_123".to_string())
        );
    }

    #[test]
    fn test_extract_stripe_event_id_not_stripe() {
        let payload = json!({ "id": "something" });
        assert_eq!(extract_stripe_event_id(&payload, "github"), None);
    }

    #[test]
    fn test_extract_event_type_stripe() {
        let payload = json!({ "type": "invoice.payment_succeeded" });
        assert_eq!(
            extract_event_type(&payload, "stripe"),
            Some("invoice.payment_succeeded".to_string())
        );
    }

    #[test]
    fn test_extract_event_type_unknown() {
        let payload = json!({ "event_type": "order.placed" });
        assert_eq!(
            extract_event_type(&payload, "custom"),
            Some("order.placed".to_string())
        );
    }
}
