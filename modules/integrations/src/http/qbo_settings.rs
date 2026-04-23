//! HTTP handlers for QBO webhook verifier token admin API.
//!
//! Routes:
//!   POST /api/integrations/qbo/webhook-token         — set per-tenant verifier token
//!   GET  /api/integrations/qbo/webhook-token/status  — check configuration status

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::domain::webhooks::secret_store;
use crate::AppState;
use platform_sdk::extract_tenant;

#[derive(Debug, Deserialize)]
pub struct SetWebhookTokenRequest {
    pub realm_id: String,
    pub token: String,
}

#[derive(Debug, Deserialize)]
pub struct TokenStatusQuery {
    pub realm_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TokenStatusResponse {
    pub configured: bool,
    pub last_set_at: Option<String>,
}

pub async fn set_webhook_token(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(body): Json<SetWebhookTokenRequest>,
) -> impl IntoResponse {
    if body.realm_id.is_empty() {
        return ApiError::new(400, "validation_error", "realm_id must not be empty")
            .into_response();
    }
    if body.token.is_empty() {
        return ApiError::new(400, "validation_error", "token must not be empty").into_response();
    }

    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    let connected: Option<(String,)> = match sqlx::query_as(
        r#"
        SELECT realm_id FROM integrations_oauth_connections
        WHERE app_id = $1 AND realm_id = $2 AND provider = 'quickbooks'
        "#,
    )
    .bind(&app_id)
    .bind(&body.realm_id)
    .fetch_optional(&state.pool)
    .await
    {
        Ok(row) => row,
        Err(e) => {
            tracing::error!(error = %e, "DB error looking up OAuth connection");
            return ApiError::internal("Internal database error").into_response();
        }
    };

    if connected.is_none() {
        return ApiError::not_found("No connected QuickBooks account for that realm_id")
            .into_response();
    }

    match secret_store::upsert_token(
        &state.pool,
        &app_id,
        &body.realm_id,
        &body.token,
        &state.webhooks_key,
    )
    .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "Failed to upsert webhook token");
            ApiError::internal("Internal error").into_response()
        }
    }
}

pub async fn webhook_token_status(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(params): Query<TokenStatusQuery>,
) -> impl IntoResponse {
    let realm_id = match params.realm_id.as_deref() {
        Some(r) if !r.is_empty() => r.to_string(),
        _ => {
            return ApiError::new(
                400,
                "validation_error",
                "realm_id query param is required and must not be empty",
            )
            .into_response();
        }
    };

    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    let row: Option<(chrono::DateTime<chrono::Utc>,)> = match sqlx::query_as(
        r#"
        SELECT configured_at FROM integrations_qbo_webhook_secrets
        WHERE app_id = $1 AND realm_id = $2
        "#,
    )
    .bind(&app_id)
    .bind(&realm_id)
    .fetch_optional(&state.pool)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "DB error checking webhook token status");
            return ApiError::internal("Internal database error").into_response();
        }
    };

    let (configured, last_set_at) = match row {
        Some((ts,)) => (true, Some(ts.to_rfc3339())),
        None => (false, None),
    };

    Json(TokenStatusResponse {
        configured,
        last_set_at,
    })
    .into_response()
}
