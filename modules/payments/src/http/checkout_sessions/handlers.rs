//! Customer-facing checkout session endpoints
//!
//! Exposes a Tilled.js-compatible checkout flow. Platform owns Tilled integration
//! — product apps never call Tilled directly.
//!
//! Endpoints:
//!   POST /api/payments/checkout-sessions            — create session, return client_secret
//!   GET  /api/payments/checkout-sessions/:id        — full session data (no secrets)
//!   POST /api/payments/checkout-sessions/:id/present — idempotent: created → presented
//!   GET  /api/payments/checkout-sessions/:id/status  — lightweight status poll (no secret)
//!   POST /api/payments/webhook/tilled               — Tilled webhook callbacks
//!
//! State machine: created → presented → completed | failed | canceled | expired

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Extension, Json,
};
use security::VerifiedClaims;
use std::sync::Arc;
use uuid::Uuid;

use crate::webhook_signature::{validate_webhook_signature, WebhookSource};

use super::session_logic::{
    create_tilled_payment_intent, poll_tilled_intent_status, validate_https_url, ApiError,
    CheckoutSessionStatusResponse, CreateCheckoutSessionRequest, CreateCheckoutSessionResponse,
    SessionStatusPollResponse,
};

// ============================================================================
// POST /api/payments/checkout-sessions
// ============================================================================

pub async fn create_checkout_session(
    State(state): State<Arc<crate::AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<CreateCheckoutSessionRequest>,
) -> Result<Json<CreateCheckoutSessionResponse>, ApiError> {
    let tenant_id = extract_tenant(&claims)?;
    req.tenant_id = tenant_id;

    if req.invoice_id.is_empty() || req.currency.is_empty() {
        return Err(ApiError {
            status: StatusCode::BAD_REQUEST,
            message: "invoice_id and currency are required".to_string(),
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
            .map_err(|e| {
                tracing::error!("Tilled API error: {}", e);
                ApiError {
                    status: StatusCode::BAD_GATEWAY,
                    message: "Payment processor error".to_string(),
                }
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
            (invoice_id, tenant_id, amount_minor, currency, processor_payment_id, return_url, cancel_url)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        RETURNING id
        "#,
    )
    .bind(&req.invoice_id)
    .bind(&req.tenant_id)
    .bind(req.amount)
    .bind(&req.currency)
    .bind(&pi_id)
    .bind(&req.return_url)
    .bind(&req.cancel_url)
    .fetch_one(&state.pool)
    .await
    .map_err(|e| {
        tracing::error!("Failed to insert checkout session: {}", e);
        ApiError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: "Internal database error".to_string(),
        }
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
    claims: Option<Extension<VerifiedClaims>>,
) -> Result<Json<CheckoutSessionStatusResponse>, ApiError> {
    let tenant_id = extract_tenant(&claims)?;

    #[derive(sqlx::FromRow)]
    struct SessionRow {
        status: String,
        processor_payment_id: String,
        invoice_id: String,
        tenant_id: String,
        amount_minor: i32,
        currency: String,
        return_url: Option<String>,
        cancel_url: Option<String>,
    }

    let row: Option<SessionRow> = sqlx::query_as(
        r#"SELECT status, processor_payment_id, invoice_id, tenant_id,
                  amount_minor, currency, return_url, cancel_url
           FROM checkout_sessions WHERE id = $1 AND tenant_id = $2"#,
    )
    .bind(session_id)
    .bind(&tenant_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| {
        tracing::error!("Database error: {}", e);
        ApiError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: "Internal database error".to_string(),
        }
    })?;

    let session = row.ok_or_else(|| ApiError {
        status: StatusCode::NOT_FOUND,
        message: format!("Checkout session not found: {}", session_id),
    })?;

    // For non-terminal sessions, poll Tilled for live status
    let is_non_terminal = matches!(session.status.as_str(), "created" | "presented");
    let status = if is_non_terminal {
        if let (Some(api_key), Some(account_id)) = (
            state.tilled_api_key.as_deref(),
            state.tilled_account_id.as_deref(),
        ) {
            match poll_tilled_intent_status(api_key, account_id, &session.processor_payment_id)
                .await
            {
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
    claims: Option<Extension<VerifiedClaims>>,
) -> Result<StatusCode, ApiError> {
    let tenant_id = extract_tenant(&claims)?;

    let rows = sqlx::query(
        "UPDATE checkout_sessions \
         SET status = 'presented', presented_at = NOW(), updated_at = NOW() \
         WHERE id = $1 AND status = 'created' AND tenant_id = $2",
    )
    .bind(session_id)
    .bind(&tenant_id)
    .execute(&state.pool)
    .await
    .map_err(|e| {
        tracing::error!("Database error updating checkout session: {}", e);
        ApiError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: "Internal database error".to_string(),
        }
    })?
    .rows_affected();

    if rows == 0 {
        // 0 rows: either already in a later state (idempotent) or session not found
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM checkout_sessions WHERE id = $1 AND tenant_id = $2)",
        )
        .bind(session_id)
        .bind(&tenant_id)
        .fetch_one(&state.pool)
        .await
        .map_err(|e| {
            tracing::error!("Database error checking session existence: {}", e);
            ApiError {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                message: "Internal database error".to_string(),
            }
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
    claims: Option<Extension<VerifiedClaims>>,
) -> Result<Json<SessionStatusPollResponse>, ApiError> {
    let tenant_id = extract_tenant(&claims)?;

    let status: Option<String> =
        sqlx::query_scalar("SELECT status FROM checkout_sessions WHERE id = $1 AND tenant_id = $2")
            .bind(session_id)
            .bind(&tenant_id)
            .fetch_optional(&state.pool)
            .await
            .map_err(|e| {
                tracing::error!("Database error polling session status: {}", e);
                ApiError {
                    status: StatusCode::INTERNAL_SERVER_ERROR,
                    message: "Internal database error".to_string(),
                }
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
            v.to_str()
                .ok()
                .map(|val| (k.as_str().to_lowercase(), val.to_string()))
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

    validate_webhook_signature(WebhookSource::Tilled, &header_map, &body, &secrets).map_err(
        |e| ApiError {
            status: StatusCode::UNAUTHORIZED,
            message: format!("Webhook signature invalid: {}", e),
        },
    )?;

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
    .map_err(|e| {
        tracing::error!("Database error in webhook handler: {}", e);
        ApiError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: "Internal database error".to_string(),
        }
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
// Auth helper
// ============================================================================

pub fn extract_tenant(claims: &Option<Extension<VerifiedClaims>>) -> Result<String, ApiError> {
    match claims {
        Some(Extension(c)) => Ok(c.tenant_id.to_string()),
        None => Err(ApiError {
            status: StatusCode::UNAUTHORIZED,
            message: "Missing or invalid authentication".to_string(),
        }),
    }
}
