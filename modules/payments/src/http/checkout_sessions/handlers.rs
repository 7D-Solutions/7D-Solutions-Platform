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
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use std::sync::Arc;
use uuid::Uuid;

use crate::webhook_signature::{validate_webhook_signature, WebhookSource};

use platform_sdk::extract_tenant;
use super::repo;
use super::session_logic::{
    create_tilled_payment_intent, poll_tilled_intent_status, validate_https_url,
    CheckoutSessionStatusResponse, CreateCheckoutSessionRequest, CreateCheckoutSessionResponse,
    SessionStatusPollResponse,
};

// ============================================================================
// Helpers
// ============================================================================

fn with_request_id(err: ApiError, ctx: &Option<Extension<TracingContext>>) -> ApiError {
    match ctx {
        Some(Extension(c)) => {
            if let Some(tid) = &c.trace_id {
                err.with_request_id(tid.clone())
            } else {
                err
            }
        }
        None => err,
    }
}

// ============================================================================
// POST /api/payments/checkout-sessions
// ============================================================================

#[utoipa::path(
    post,
    path = "/api/payments/checkout-sessions",
    tag = "Checkout Sessions",
    request_body = CreateCheckoutSessionRequest,
    responses(
        (status = 200, description = "Session created", body = CreateCheckoutSessionResponse),
        (status = 400, description = "Invalid request", body = ApiError),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 502, description = "Payment processor error", body = ApiError),
    ),
    security(("bearer" = ["PAYMENTS_MUTATE"]))
)]
pub async fn create_checkout_session(
    State(state): State<Arc<crate::AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<CreateCheckoutSessionRequest>,
) -> Result<Json<CreateCheckoutSessionResponse>, ApiError> {
    let tenant_id = extract_tenant(&claims).map_err(|e| with_request_id(e, &tracing_ctx))?;
    req.tenant_id = tenant_id;

    if req.invoice_id.is_empty() || req.currency.is_empty() {
        return Err(with_request_id(
            ApiError::bad_request("invoice_id and currency are required"),
            &tracing_ctx,
        ));
    }
    if req.amount <= 0 {
        return Err(with_request_id(
            ApiError::bad_request("amount must be positive"),
            &tracing_ctx,
        ));
    }

    // Strict URL validation: absolute HTTPS only, no injection
    if let Some(ref url) = req.return_url {
        if !validate_https_url(url) {
            return Err(with_request_id(
                ApiError::bad_request("return_url must be an absolute HTTPS URL"),
                &tracing_ctx,
            ));
        }
    }
    if let Some(ref url) = req.cancel_url {
        if !validate_https_url(url) {
            return Err(with_request_id(
                ApiError::bad_request("cancel_url must be an absolute HTTPS URL"),
                &tracing_ctx,
            ));
        }
    }

    // Effective idempotency key: explicit key if provided, else invoice_id
    let idem_key = req
        .idempotency_key
        .as_deref()
        .unwrap_or(&req.invoice_id)
        .to_string();

    // Check for existing session with same (tenant_id, idempotency_key)
    let existing = repo::find_session_by_idempotency_key(&state.pool, &req.tenant_id, &idem_key)
        .await
        .map_err(|e| {
            tracing::error!("Failed to check existing checkout session: {}", e);
            with_request_id(
                ApiError::internal("Internal database error"),
                &tracing_ctx,
            )
        })?;

    if let Some(session) = existing {
        tracing::info!(
            session_id = %session.id,
            idempotency_key = %idem_key,
            tenant_id = %req.tenant_id,
            "Returning existing checkout session (idempotent)"
        );
        return Ok(Json(CreateCheckoutSessionResponse {
            session_id: session.id.to_string(),
            payment_intent_id: session.processor_payment_id,
            client_secret: session.client_secret.unwrap_or_default(),
        }));
    }

    let (pi_id, client_secret) = if let (Some(api_key), Some(account_id)) = (
        state.tilled_api_key.as_deref(),
        state.tilled_account_id.as_deref(),
    ) {
        create_tilled_payment_intent(api_key, account_id, req.amount, &req.currency)
            .await
            .map_err(|e| {
                tracing::error!("Tilled API error: {}", e);
                with_request_id(
                    ApiError::new(502, "bad_gateway", "Payment processor error"),
                    &tracing_ctx,
                )
            })?
    } else {
        // Mock provider: generate fake IDs for dev/test
        let pi_id = format!("mock_pi_{}", Uuid::new_v4().simple());
        let secret = format!("{}_secret_{}", pi_id, Uuid::new_v4().simple());
        (pi_id, secret)
    };

    let insert_result = repo::insert_checkout_session(
        &state.pool,
        &req.invoice_id,
        &req.tenant_id,
        req.amount,
        &req.currency,
        &pi_id,
        &client_secret,
        &idem_key,
        &req.return_url,
        &req.cancel_url,
    )
    .await;

    match insert_result {
        Ok(session_id) => {
            tracing::info!(
                session_id = %session_id,
                invoice_id = %req.invoice_id,
                tenant_id = %req.tenant_id,
                pi_id = %pi_id,
                idempotency_key = %idem_key,
                "Checkout session created"
            );

            Ok(Json(CreateCheckoutSessionResponse {
                session_id: session_id.to_string(),
                payment_intent_id: pi_id,
                client_secret,
            }))
        }
        Err(e) if e.to_string().contains("uq_checkout_sessions_tenant_idem_key")
            || e.to_string().contains("duplicate key") =>
        {
            tracing::info!(
                idempotency_key = %idem_key,
                tenant_id = %req.tenant_id,
                "Race condition on idempotent insert — fetching existing session"
            );
            // Race condition fallback: fetch the session that won the insert race
            let winner =
                repo::find_session_by_idempotency_key(&state.pool, &req.tenant_id, &idem_key)
                    .await
                    .map_err(|e2| {
                        tracing::error!("Failed to fetch race-winner session: {}", e2);
                        with_request_id(
                            ApiError::internal("Internal database error"),
                            &tracing_ctx,
                        )
                    })?
                    .ok_or_else(|| {
                        with_request_id(
                            ApiError::internal("Internal database error"),
                            &tracing_ctx,
                        )
                    })?;

            Ok(Json(CreateCheckoutSessionResponse {
                session_id: winner.id.to_string(),
                payment_intent_id: winner.processor_payment_id,
                client_secret: winner.client_secret.unwrap_or_default(),
            }))
        }
        Err(e) => {
            tracing::error!("Failed to insert checkout session: {}", e);
            Err(with_request_id(
                ApiError::internal("Internal database error"),
                &tracing_ctx,
            ))
        }
    }
}

// ============================================================================
// GET /api/payments/checkout-sessions/:id
// ============================================================================

#[utoipa::path(
    get,
    path = "/api/payments/checkout-sessions/{id}",
    tag = "Checkout Sessions",
    params(("id" = Uuid, Path, description = "Checkout session ID")),
    responses(
        (status = 200, description = "Session details", body = CheckoutSessionStatusResponse),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 404, description = "Session not found", body = ApiError),
    ),
    security(("bearer" = ["PAYMENTS_MUTATE"]))
)]
pub async fn get_checkout_session(
    State(state): State<Arc<crate::AppState>>,
    Path(session_id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> Result<Json<CheckoutSessionStatusResponse>, ApiError> {
    let tenant_id = extract_tenant(&claims).map_err(|e| with_request_id(e, &tracing_ctx))?;

    let row = repo::find_session_details(&state.pool, session_id, &tenant_id)
        .await
        .map_err(|e| {
            tracing::error!("Database error: {}", e);
            with_request_id(
                ApiError::internal("Internal database error"),
                &tracing_ctx,
            )
        })?;

    let session = row.ok_or_else(|| {
        with_request_id(
            ApiError::not_found(format!("Checkout session not found: {}", session_id)),
            &tracing_ctx,
        )
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
                    let _ =
                        repo::update_session_status(&state.pool, session_id, &live_status).await;
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

#[utoipa::path(
    post,
    path = "/api/payments/checkout-sessions/{id}/present",
    tag = "Checkout Sessions",
    params(("id" = Uuid, Path, description = "Checkout session ID")),
    responses(
        (status = 200, description = "Session presented (or already in later state)"),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 404, description = "Session not found", body = ApiError),
    ),
    security(("bearer" = ["PAYMENTS_MUTATE"]))
)]
pub async fn present_checkout_session(
    State(state): State<Arc<crate::AppState>>,
    Path(session_id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> Result<StatusCode, ApiError> {
    let tenant_id = extract_tenant(&claims).map_err(|e| with_request_id(e, &tracing_ctx))?;

    let rows = repo::present_session(&state.pool, session_id, &tenant_id)
        .await
        .map_err(|e| {
            tracing::error!("Database error updating checkout session: {}", e);
            with_request_id(
                ApiError::internal("Internal database error"),
                &tracing_ctx,
            )
        })?;

    if rows == 0 {
        // 0 rows: either already in a later state (idempotent) or session not found
        let exists = repo::session_exists(&state.pool, session_id, &tenant_id)
            .await
            .map_err(|e| {
                tracing::error!("Database error checking session existence: {}", e);
                with_request_id(
                    ApiError::internal("Internal database error"),
                    &tracing_ctx,
                )
            })?;

        if !exists {
            return Err(with_request_id(
                ApiError::not_found(format!("Checkout session not found: {}", session_id)),
                &tracing_ctx,
            ));
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

#[utoipa::path(
    get,
    path = "/api/payments/checkout-sessions/{id}/status",
    tag = "Checkout Sessions",
    params(("id" = Uuid, Path, description = "Checkout session ID")),
    responses(
        (status = 200, description = "Session status", body = SessionStatusPollResponse),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 404, description = "Session not found", body = ApiError),
    ),
    security(("bearer" = ["PAYMENTS_MUTATE"]))
)]
pub async fn poll_checkout_session_status(
    State(state): State<Arc<crate::AppState>>,
    Path(session_id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> Result<Json<SessionStatusPollResponse>, ApiError> {
    let tenant_id = extract_tenant(&claims).map_err(|e| with_request_id(e, &tracing_ctx))?;

    let status = repo::poll_session_status(&state.pool, session_id, &tenant_id)
        .await
        .map_err(|e| {
            tracing::error!("Database error polling session status: {}", e);
            with_request_id(
                ApiError::internal("Internal database error"),
                &tracing_ctx,
            )
        })?;

    let status = status.ok_or_else(|| {
        with_request_id(
            ApiError::not_found(format!("Checkout session not found: {}", session_id)),
            &tracing_ctx,
        )
    })?;

    Ok(Json(SessionStatusPollResponse {
        session_id: session_id.to_string(),
        status,
    }))
}

// ============================================================================
// POST /api/payments/webhook/tilled  — Tilled PSP callbacks
// NOTE: This endpoint is UNAUTHENTICATED — webhook signature is the only guard.
// ============================================================================

#[utoipa::path(
    post,
    path = "/api/payments/webhook/tilled",
    tag = "Webhooks",
    responses(
        (status = 200, description = "Webhook processed"),
        (status = 400, description = "Invalid JSON body", body = ApiError),
        (status = 401, description = "Invalid signature", body = ApiError),
    ),
)]
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
        |e| ApiError::unauthorized(format!("Webhook signature invalid: {}", e)),
    )?;

    let event: serde_json::Value = serde_json::from_slice(&body).map_err(|_| {
        ApiError::bad_request("Invalid JSON body")
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
    let rows_updated = repo::update_status_by_processor_id(&state.pool, pi_id, new_status)
        .await
        .map_err(|e| {
            tracing::error!("Database error in webhook handler: {}", e);
            ApiError::internal("Internal database error")
        })?;

    tracing::info!(
        event_type = event_type,
        pi_id = pi_id,
        new_status = new_status,
        rows_updated = rows_updated,
        "Tilled webhook processed"
    );

    Ok(StatusCode::OK)
}
