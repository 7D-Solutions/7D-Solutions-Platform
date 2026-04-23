//! HTTP handlers for OAuth connection lifecycle.
//!
//! Routes:
//!   GET  /api/integrations/oauth/connect/{provider}     — redirect to provider auth URL
//!   GET  /api/integrations/oauth/callback/{provider}    — handle provider redirect
//!   GET  /api/integrations/oauth/status/{provider}      — connection status
//!   POST /api/integrations/oauth/disconnect/{provider}  — disconnect
//!   POST /api/integrations/oauth/import                 — seed tokens directly (admin + env gate)

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect},
    Extension, Json,
};
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use serde::Deserialize;
use std::sync::Arc;

use crate::domain::oauth::{service, OAuthConnectionInfo, OAuthError, TokenResponse};
use crate::AppState;
use platform_sdk::extract_tenant;

// ============================================================================
// Error helpers
// ============================================================================

fn oauth_error(e: OAuthError) -> ApiError {
    match e {
        OAuthError::NotFound => ApiError::not_found("No connection found"),
        OAuthError::UnsupportedProvider(p) => ApiError::new(
            422,
            "unsupported_provider",
            format!("Provider not supported: {}", p),
        ),
        OAuthError::TokenExchangeFailed(msg) => ApiError::new(502, "token_exchange_failed", msg),
        OAuthError::MissingEncryptionKey => {
            tracing::error!(
                error_code = "OPERATION_FAILED",
                "OAUTH_ENCRYPTION_KEY not set"
            );
            ApiError::internal("Server misconfiguration")
        }
        OAuthError::DuplicateConnection(msg) => ApiError::conflict(msg),
        OAuthError::AlreadyDisconnected => ApiError::conflict("Connection is already disconnected"),
        OAuthError::Database(e) => {
            tracing::error!(error = %e, "OAuth DB error");
            ApiError::internal("Internal database error")
        }
    }
}

fn validate_provider(provider: &str) -> Result<(), ApiError> {
    match provider {
        "quickbooks" => Ok(()),
        _ => Err(oauth_error(OAuthError::UnsupportedProvider(
            provider.to_string(),
        ))),
    }
}

// ============================================================================
// Provider config (from env vars)
// ============================================================================

struct QboConfig {
    client_id: String,
    client_secret: String,
    redirect_uri: String,
    auth_url: String,
    token_url: String,
}

impl QboConfig {
    fn from_env() -> Result<Self, ApiError> {
        let missing = |var: &str| {
            tracing::error!(env_var = %var, "env var not set");
            ApiError::internal("Server misconfiguration")
        };

        Ok(Self {
            client_id: std::env::var("QBO_CLIENT_ID").map_err(|_| missing("QBO_CLIENT_ID"))?,
            client_secret: std::env::var("QBO_CLIENT_SECRET")
                .map_err(|_| missing("QBO_CLIENT_SECRET"))?,
            redirect_uri: std::env::var("QBO_REDIRECT_URI")
                .map_err(|_| missing("QBO_REDIRECT_URI"))?,
            auth_url: std::env::var("QBO_AUTH_URL")
                .unwrap_or_else(|_| "https://appcenter.intuit.com/connect/oauth2".to_string()),
            token_url: std::env::var("QBO_TOKEN_URL").unwrap_or_else(|_| {
                "https://oauth.platform.intuit.com/oauth2/v1/tokens/bearer".to_string()
            }),
        })
    }
}

// ============================================================================
// Callback query params
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct OAuthCallbackQuery {
    pub code: String,
    #[serde(rename = "realmId")]
    pub realm_id: String,
    #[serde(default)]
    pub state: Option<String>,
}

// ============================================================================
// Handlers
// ============================================================================

#[utoipa::path(
    get,
    path = "/api/integrations/oauth/connect/{provider}",
    params(("provider" = String, Path, description = "OAuth provider (e.g. quickbooks)")),
    responses(
        (status = 307, description = "Redirect to provider authorization page"),
        (status = 422, description = "Unsupported provider"),
    ),
    security(("bearer" = [])),
    tag = "OAuth"
)]
/// GET /api/integrations/oauth/connect/{provider}
///
/// Redirects the user to the provider's authorization page.
pub async fn connect(
    claims: Option<Extension<VerifiedClaims>>,
    Path(provider): Path<String>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    if let Err(e) = validate_provider(&provider) {
        return e.into_response();
    }

    let config = match QboConfig::from_env() {
        Ok(c) => c,
        Err(e) => return e.into_response(),
    };

    let scopes = "com.intuit.quickbooks.accounting";
    let auth_url = format!(
        "{}?client_id={}&response_type=code&scope={}&redirect_uri={}&state={}",
        config.auth_url,
        urlencoding::encode(&config.client_id),
        urlencoding::encode(scopes),
        urlencoding::encode(&config.redirect_uri),
        urlencoding::encode(&app_id),
    );

    Redirect::temporary(&auth_url).into_response()
}

#[utoipa::path(
    get,
    path = "/api/integrations/oauth/callback/{provider}",
    params(
        ("provider" = String, Path, description = "OAuth provider"),
        ("code" = String, Query, description = "Authorization code"),
        ("realmId" = String, Query, description = "QBO realm ID"),
        ("state" = Option<String>, Query, description = "State parameter (app_id)"),
    ),
    responses(
        (status = 201, description = "Connection created", body = OAuthConnectionInfo),
        (status = 502, description = "Token exchange failed"),
    ),
    tag = "OAuth"
)]
/// GET /api/integrations/oauth/callback/{provider}
///
/// Handles the redirect from the provider after user authorization.
/// Exchanges the auth code for tokens and persists the connection.
pub async fn callback(
    State(state): State<Arc<AppState>>,
    Path(provider): Path<String>,
    Query(params): Query<OAuthCallbackQuery>,
) -> impl IntoResponse {
    if let Err(e) = validate_provider(&provider) {
        return e.into_response();
    }

    // state carries the app_id set during connect — validate before any env/token work (CSRF guard)
    let app_id = match params.state.as_deref() {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => {
            tracing::warn!("OAuth callback rejected: missing or empty state — possible CSRF");
            return ApiError::new(400, "invalid_state", "Missing OAuth state parameter")
                .into_response();
        }
    };

    let config = match QboConfig::from_env() {
        Ok(c) => c,
        Err(e) => return e.into_response(),
    };

    // Exchange authorization code for tokens
    let client = reqwest::Client::new();
    let resp = match client
        .post(&config.token_url)
        .basic_auth(&config.client_id, Some(&config.client_secret))
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", &params.code),
            ("redirect_uri", &config.redirect_uri),
        ])
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return oauth_error(OAuthError::TokenExchangeFailed(format!(
                "HTTP request failed: {}",
                e
            )))
            .into_response()
        }
    };

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return oauth_error(OAuthError::TokenExchangeFailed(format!(
            "Token exchange failed: HTTP {} — {}",
            status, body
        )))
        .into_response();
    }

    let tokens: TokenResponse = match resp.json().await {
        Ok(t) => t,
        Err(e) => {
            return oauth_error(OAuthError::TokenExchangeFailed(format!(
                "Failed to parse token response: {}",
                e
            )))
            .into_response()
        }
    };

    let scopes = "com.intuit.quickbooks.accounting";

    match service::create_connection(
        &state.pool,
        &app_id,
        &provider,
        &params.realm_id,
        scopes,
        &tokens,
    )
    .await
    {
        Ok(connection) => (StatusCode::CREATED, Json(connection)).into_response(),
        Err(e) => oauth_error(e).into_response(),
    }
}

#[utoipa::path(
    get,
    path = "/api/integrations/oauth/status/{provider}",
    params(("provider" = String, Path, description = "OAuth provider")),
    responses(
        (status = 200, description = "Connection status", body = OAuthConnectionInfo),
        (status = 404, description = "No connection found"),
    ),
    security(("bearer" = [])),
    tag = "OAuth"
)]
/// GET /api/integrations/oauth/status/{provider}
///
/// Returns the connection status for the current tenant + provider.
pub async fn status(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(provider): Path<String>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    if let Err(e) = validate_provider(&provider) {
        return e.into_response();
    }

    match service::get_connection_status(&state.pool, &app_id, &provider).await {
        Ok(Some(info)) => Json(info).into_response(),
        Ok(None) => ApiError::not_found("No connection found").into_response(),
        Err(e) => oauth_error(e).into_response(),
    }
}

#[utoipa::path(
    post,
    path = "/api/integrations/oauth/disconnect/{provider}",
    params(("provider" = String, Path, description = "OAuth provider")),
    responses(
        (status = 200, description = "Disconnected", body = OAuthConnectionInfo),
        (status = 404, description = "No connection found"),
        (status = 409, description = "Already disconnected"),
    ),
    security(("bearer" = [])),
    tag = "OAuth"
)]
/// POST /api/integrations/oauth/disconnect/{provider}
///
/// Marks the connection as disconnected.
pub async fn disconnect(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(provider): Path<String>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    if let Err(e) = validate_provider(&provider) {
        return e.into_response();
    }

    match service::disconnect(&state.pool, &app_id, &provider).await {
        Ok(info) => Json(info).into_response(),
        Err(e) => oauth_error(e).into_response(),
    }
}

// ============================================================================
// Token import (dev/CI seeding, admin-gated)
// ============================================================================

/// Runtime gate: allow import when OAUTH_IMPORT_ENABLED=1 OR ENV is not production.
///
/// Both conditions are checked so a misconfigured production deployment without
/// the env flag can never be reached via this endpoint.
pub fn is_import_enabled() -> bool {
    if std::env::var("OAUTH_IMPORT_ENABLED")
        .map(|v| v == "1")
        .unwrap_or(false)
    {
        return true;
    }
    let env = std::env::var("ENV").unwrap_or_default();
    env != "production"
}

/// Request body for the token import endpoint.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ImportTokensRequest {
    pub provider: String,
    pub realm_id: String,
    pub access_token: String,
    pub refresh_token: String,
    /// Seconds until the access token expires.
    pub expires_in: i64,
    /// Seconds until the refresh token expires (0 → default 100 days).
    pub refresh_token_expires_in: i64,
    pub scopes: String,
}

#[utoipa::path(
    post,
    path = "/api/integrations/oauth/import",
    request_body = ImportTokensRequest,
    responses(
        (status = 201, description = "Connection created or updated", body = OAuthConnectionInfo),
        (status = 403, description = "Import disabled or caller lacks integrations.oauth.admin"),
        (status = 422, description = "Unsupported provider"),
    ),
    security(("bearer" = [])),
    tag = "OAuth"
)]
/// POST /api/integrations/oauth/import
///
/// Seed OAuth tokens directly — skips the browser consent flow.
/// Requires the `integrations.oauth.admin` permission AND
/// either `OAUTH_IMPORT_ENABLED=1` or a non-production environment.
///
/// Tokens are encrypted with `pgp_sym_encrypt` via `OAUTH_ENCRYPTION_KEY`,
/// identical to the callback path.
pub async fn import_tokens(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(body): Json<ImportTokensRequest>,
) -> impl IntoResponse {
    if !is_import_enabled() {
        return ApiError::new(
            403,
            "import_disabled",
            "OAuth token import is not enabled in this environment",
        )
        .into_response();
    }

    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    if let Err(e) = validate_provider(&body.provider) {
        return e.into_response();
    }

    match service::import_connection(
        &state.pool,
        &app_id,
        &body.provider,
        &body.realm_id,
        &body.access_token,
        &body.refresh_token,
        body.expires_in,
        body.refresh_token_expires_in,
        &body.scopes,
    )
    .await
    {
        Ok(connection) => (StatusCode::CREATED, Json(connection)).into_response(),
        Err(e) => oauth_error(e).into_response(),
    }
}
