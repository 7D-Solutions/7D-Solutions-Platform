//! Customer-facing checkout session endpoints
//!
//! Exposes a Tilled.js-compatible checkout flow. Platform owns Tilled integration
//! — product apps never call Tilled directly.
//!
//! Endpoints:
//!   POST /api/payments/checkout-sessions            — create session, return client_secret
//!   GET  /api/payments/checkout-sessions/:id        — full session data (includes client_secret)
//!   POST /api/payments/checkout-sessions/:id/present — idempotent: created → presented
//!   GET  /api/payments/checkout-sessions/:id/status  — lightweight status poll (no secret)
//!   POST /api/payments/webhook/tilled               — Tilled webhook callbacks
//!
//! State machine: created → presented → completed | failed | canceled | expired

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::webhook_signature::{validate_webhook_signature, WebhookSource};

// ============================================================================
// Request / Response types
// ============================================================================

/// POST /api/payments/checkout-sessions request body
#[derive(Debug, Deserialize)]
pub struct CreateCheckoutSessionRequest {
    pub invoice_id: String,
    pub tenant_id: String,
    /// Amount in minor currency units (e.g. cents)
    pub amount: i32,
    pub currency: String,
    /// URL to redirect after successful payment (optional)
    pub return_url: Option<String>,
    /// URL to redirect after cancelled payment (optional)
    pub cancel_url: Option<String>,
}

/// POST /api/payments/checkout-sessions response
#[derive(Debug, Serialize)]
pub struct CreateCheckoutSessionResponse {
    pub session_id: String,
    pub payment_intent_id: String,
    /// Tilled.js client secret — pass to tilled.js confirmPayment()
    pub client_secret: String,
}

/// GET /api/payments/checkout-sessions/:id response
#[derive(Debug, Serialize)]
pub struct CheckoutSessionStatusResponse {
    pub session_id: String,
    pub status: String,
    pub payment_intent_id: String,
    pub invoice_id: String,
    pub tenant_id: String,
    pub amount: i32,
    pub currency: String,
    /// Tilled.js client secret — used by hosted pay page to init Tilled.js
    pub client_secret: String,
    /// URL to redirect after successful payment (stored at creation time)
    pub return_url: Option<String>,
    /// URL to redirect after cancelled payment (stored at creation time)
    pub cancel_url: Option<String>,
}

/// GET /api/payments/checkout-sessions/:id/status response (no secrets)
#[derive(Debug, Serialize)]
pub struct SessionStatusPollResponse {
    pub session_id: String,
    pub status: String,
}

/// Error response body
#[derive(Debug, Serialize)]
struct ErrorBody {
    pub error: String,
}

/// HTTP error wrapper
pub struct ApiError {
    status: StatusCode,
    message: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(ErrorBody { error: self.message })).into_response()
    }
}

// ============================================================================
// URL validation
// ============================================================================

/// Validate that a redirect URL is absolute HTTPS with no injection characters.
/// Enforces: https:// scheme, max 2048 chars, no control characters.
fn validate_https_url(url: &str) -> bool {
    if !url.starts_with("https://") {
        return false;
    }
    if url.len() > 2048 {
        return false;
    }
    // Reject control characters (injection prevention)
    if url.chars().any(|c| (c as u32) < 0x20) {
        return false;
    }
    true
}

// ============================================================================
// POST /api/payments/checkout-sessions
// ============================================================================

pub async fn create_checkout_session(
    State(state): State<Arc<crate::AppState>>,
    Json(req): Json<CreateCheckoutSessionRequest>,
) -> Result<Json<CreateCheckoutSessionResponse>, ApiError> {
    if req.invoice_id.is_empty() || req.tenant_id.is_empty() || req.currency.is_empty() {
        return Err(ApiError {
            status: StatusCode::BAD_REQUEST,
            message: "invoice_id, tenant_id, and currency are required".to_string(),
        });
    }
    if req.amount <= 0 {
        return Err(ApiError {
            status: StatusCode::BAD_REQUEST,
            message: "amount must be positive".to_string(),
        });
    }

    // Strict URL validation: absolute HTTPS only, no injection
    if let Some(ref url) = req.return_url {
        if !validate_https_url(url) {
            return Err(ApiError {
                status: StatusCode::BAD_REQUEST,
                message: "return_url must be an absolute HTTPS URL".to_string(),
            });
        }
    }
    if let Some(ref url) = req.cancel_url {
        if !validate_https_url(url) {
            return Err(ApiError {
                status: StatusCode::BAD_REQUEST,
                message: "cancel_url must be an absolute HTTPS URL".to_string(),
            });
        }
    }

    let (pi_id, client_secret) = if let (Some(api_key), Some(account_id)) = (
        state.tilled_api_key.as_deref(),
        state.tilled_account_id.as_deref(),
    ) {
        create_tilled_payment_intent(api_key, account_id, req.amount, &req.currency)
            .await
            .map_err(|e| ApiError {
                status: StatusCode::BAD_GATEWAY,
                message: format!("Tilled API error: {}", e),
            })?
    } else {
        // Mock provider: generate fake IDs for dev/test
        let pi_id = format!("mock_pi_{}", Uuid::new_v4().simple());
        let secret = format!("{}_secret_{}", pi_id, Uuid::new_v4().simple());
        (pi_id, secret)
    };

    let session_id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO checkout_sessions
            (invoice_id, tenant_id, amount_minor, currency, processor_payment_id, client_secret, return_url, cancel_url)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        RETURNING id
        "#,
    )
    .bind(&req.invoice_id)
    .bind(&req.tenant_id)
    .bind(req.amount)
    .bind(&req.currency)
    .bind(&pi_id)
    .bind(&client_secret)
    .bind(&req.return_url)
    .bind(&req.cancel_url)
    .fetch_one(&state.pool)
    .await
    .map_err(|e| ApiError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        message: format!("Database error: {}", e),
    })?;

    tracing::info!(
        session_id = %session_id,
        invoice_id = %req.invoice_id,
        tenant_id = %req.tenant_id,
        pi_id = %pi_id,
        "Checkout session created"
    );

    Ok(Json(CreateCheckoutSessionResponse {
        session_id: session_id.to_string(),
        payment_intent_id: pi_id,
        client_secret,
    }))
}

// ============================================================================
// GET /api/payments/checkout-sessions/:id
// ============================================================================

pub async fn get_checkout_session(
    State(state): State<Arc<crate::AppState>>,
    Path(session_id): Path<Uuid>,
) -> Result<Json<CheckoutSessionStatusResponse>, ApiError> {
    #[derive(sqlx::FromRow)]
    struct SessionRow {
        status: String,
        processor_payment_id: String,
        invoice_id: String,
        tenant_id: String,
        amount_minor: i32,
        currency: String,
        client_secret: String,
        return_url: Option<String>,
        cancel_url: Option<String>,
    }

    let row: Option<SessionRow> = sqlx::query_as(
        r#"SELECT status, processor_payment_id, invoice_id, tenant_id,
                  amount_minor, currency, client_secret, return_url, cancel_url
           FROM checkout_sessions WHERE id = $1"#,
    )
    .bind(session_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| ApiError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        message: format!("Database error: {}", e),
    })?;

    let session = row.ok_or_else(|| ApiError {
        status: StatusCode::NOT_FOUND,
        message: format!("Checkout session not found: {}", session_id),
    })?;

    // Tenant validation: session must belong to a real tenant
    if session.tenant_id.is_empty() {
        return Err(ApiError {
            status: StatusCode::NOT_FOUND,
            message: "Checkout session not found".to_string(),
        });
    }

    // For non-terminal sessions, poll Tilled for live status
    let is_non_terminal = matches!(session.status.as_str(), "created" | "presented");
    let status = if is_non_terminal {
        if let (Some(api_key), Some(account_id)) = (
            state.tilled_api_key.as_deref(),
            state.tilled_account_id.as_deref(),
        ) {
            match poll_tilled_intent_status(api_key, account_id, &session.processor_payment_id).await {
                Ok(live_status) if live_status != session.status => {
                    // Update cached status if it changed
                    let _ = sqlx::query(
                        "UPDATE checkout_sessions SET status = $1, updated_at = NOW() WHERE id = $2",
                    )
                    .bind(&live_status)
                    .bind(session_id)
                    .execute(&state.pool)
                    .await;
                    live_status
                }
                Ok(s) => s,
                Err(_) => session.status.clone(), // fall back to cached
            }
        } else {
            session.status.clone()
        }
    } else {
        session.status.clone()
    };

    Ok(Json(CheckoutSessionStatusResponse {
        session_id: session_id.to_string(),
        status,
        payment_intent_id: session.processor_payment_id,
        invoice_id: session.invoice_id,
        tenant_id: session.tenant_id,
        amount: session.amount_minor,
        currency: session.currency,
        client_secret: session.client_secret,
        return_url: session.return_url,
        cancel_url: session.cancel_url,
    }))
}

// ============================================================================
// POST /api/payments/checkout-sessions/:id/present
// Idempotent: transitions 'created' → 'presented' on hosted page load.
// If already in 'presented' or a terminal state, returns 200 (no-op).
// ============================================================================

pub async fn present_checkout_session(
    State(state): State<Arc<crate::AppState>>,
    Path(session_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let rows = sqlx::query(
        "UPDATE checkout_sessions \
         SET status = 'presented', presented_at = NOW(), updated_at = NOW() \
         WHERE id = $1 AND status = 'created'",
    )
    .bind(session_id)
    .execute(&state.pool)
    .await
    .map_err(|e| ApiError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        message: format!("Database error: {}", e),
    })?
    .rows_affected();

    if rows == 0 {
        // 0 rows: either already in a later state (idempotent) or session not found
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM checkout_sessions WHERE id = $1)",
        )
        .bind(session_id)
        .fetch_one(&state.pool)
        .await
        .map_err(|e| ApiError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: format!("Database error: {}", e),
        })?;

        if !exists {
            return Err(ApiError {
                status: StatusCode::NOT_FOUND,
                message: format!("Checkout session not found: {}", session_id),
            });
        }
        // Already presented or terminal — idempotent no-op
    }

    tracing::info!(session_id = %session_id, rows_updated = rows, "Session present called");
    Ok(StatusCode::OK)
}

// ============================================================================
// GET /api/payments/checkout-sessions/:id/status
// Lightweight status poll — does not return client_secret.
// Used by the hosted pay page for client-side status polling after payment.
// ============================================================================

pub async fn poll_checkout_session_status(
    State(state): State<Arc<crate::AppState>>,
    Path(session_id): Path<Uuid>,
) -> Result<Json<SessionStatusPollResponse>, ApiError> {
    let status: Option<String> =
        sqlx::query_scalar("SELECT status FROM checkout_sessions WHERE id = $1")
            .bind(session_id)
            .fetch_optional(&state.pool)
            .await
            .map_err(|e| ApiError {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                message: format!("Database error: {}", e),
            })?;

    let status = status.ok_or_else(|| ApiError {
        status: StatusCode::NOT_FOUND,
        message: format!("Checkout session not found: {}", session_id),
    })?;

    Ok(Json(SessionStatusPollResponse {
        session_id: session_id.to_string(),
        status,
    }))
}

// ============================================================================
// POST /api/payments/webhook/tilled  — Tilled PSP callbacks
// ============================================================================

pub async fn tilled_webhook(
    State(state): State<Arc<crate::AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<StatusCode, ApiError> {
    // Build lowercase header map for signature check
    let header_map: std::collections::HashMap<String, String> = headers
        .iter()
        .filter_map(|(k, v)| {
            v.to_str().ok().map(|val| (k.as_str().to_lowercase(), val.to_string()))
        })
        .collect();

    let secret_str = state.tilled_webhook_secret.as_deref().unwrap_or("");
    let prev_str = state.tilled_webhook_secret_prev.as_deref().unwrap_or("");
    let mut secrets: Vec<&str> = Vec::with_capacity(2);
    if !secret_str.is_empty() {
        secrets.push(secret_str);
    }
    if !prev_str.is_empty() {
        secrets.push(prev_str);
    }

    validate_webhook_signature(WebhookSource::Tilled, &header_map, &body, &secrets).map_err(|e| {
        ApiError {
            status: StatusCode::UNAUTHORIZED,
            message: format!("Webhook signature invalid: {}", e),
        }
    })?;

    let event: serde_json::Value = serde_json::from_slice(&body).map_err(|_| ApiError {
        status: StatusCode::BAD_REQUEST,
        message: "Invalid JSON body".to_string(),
    })?;

    let event_type = event["type"].as_str().unwrap_or("");
    let pi_id = event["data"]["object"]["id"].as_str().unwrap_or("");

    if pi_id.is_empty() {
        // Unknown event type or no payment_intent — ack and ignore
        return Ok(StatusCode::OK);
    }

    let new_status = match event_type {
        "payment_intent.succeeded" => "completed",
        "payment_intent.payment_failed" => "failed",
        "payment_intent.canceled" => "canceled",
        _ => return Ok(StatusCode::OK), // ack unknown events
    };

    // Idempotent: only transition from non-terminal states.
    // If already in completed/failed/canceled/expired, UPDATE matches 0 rows — no-op.
    let rows_updated = sqlx::query(
        "UPDATE checkout_sessions \
         SET status = $1, updated_at = NOW() \
         WHERE processor_payment_id = $2 \
         AND status IN ('created', 'presented')",
    )
    .bind(new_status)
    .bind(pi_id)
    .execute(&state.pool)
    .await
    .map_err(|e| ApiError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        message: format!("Database error: {}", e),
    })?
    .rows_affected();

    tracing::info!(
        event_type = event_type,
        pi_id = pi_id,
        new_status = new_status,
        rows_updated = rows_updated,
        "Tilled webhook processed"
    );

    Ok(StatusCode::OK)
}

// ============================================================================
// Tilled API helpers
// ============================================================================

/// Create a PaymentIntent in `requires_payment_method` state (no confirmation).
/// Returns (payment_intent_id, client_secret).
async fn create_tilled_payment_intent(
    api_key: &str,
    account_id: &str,
    amount: i32,
    currency: &str,
) -> anyhow::Result<(String, String)> {
    let client = reqwest::Client::new();
    let url = format!(
        "https://api.tilled.com/v1/payment-intents?tilled_account={}",
        account_id
    );
    let body = serde_json::json!({
        "amount": amount,
        "currency": currency.to_lowercase(),
        "payment_method_types": ["card"],
        "capture_method": "automatic",
    });

    let resp = client
        .post(&url)
        .header("tilled-account", account_id)
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("Tilled create PI failed ({}): {}", status, text);
    }

    let pi: serde_json::Value = resp.json().await?;
    let pi_id = pi["id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Tilled response missing id"))?
        .to_string();
    let client_secret = pi["client_secret"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Tilled response missing client_secret"))?
        .to_string();

    Ok((pi_id, client_secret))
}

/// Query Tilled for current PaymentIntent status and map to our session status string.
async fn poll_tilled_intent_status(
    api_key: &str,
    account_id: &str,
    pi_id: &str,
) -> anyhow::Result<String> {
    let client = reqwest::Client::new();
    let url = format!(
        "https://api.tilled.com/v1/payment-intents/{}?tilled_account={}",
        pi_id, account_id
    );

    let resp = client
        .get(&url)
        .header("tilled-account", account_id)
        .bearer_auth(api_key)
        .send()
        .await?;

    if !resp.status().is_success() {
        anyhow::bail!("Tilled query PI failed ({})", resp.status());
    }

    let pi: serde_json::Value = resp.json().await?;
    let status = match pi["status"].as_str().unwrap_or("unknown") {
        "succeeded" => "completed",
        "canceled" => "canceled",
        "requires_payment_method" | "requires_action" | "processing" | "requires_confirmation" => {
            "presented"
        }
        _ => "created",
    };
    Ok(status.to_string())
}
