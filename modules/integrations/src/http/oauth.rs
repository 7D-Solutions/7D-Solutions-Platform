//! HTTP handlers for OAuth connection lifecycle.
//!
//! Routes:
//!   GET  /api/integrations/oauth/connect/{provider}     — redirect to provider auth URL
//!   GET  /api/integrations/oauth/callback/{provider}    — handle provider redirect
//!   GET  /api/integrations/oauth/status/{provider}      — connection status
//!   POST /api/integrations/oauth/disconnect/{provider}  — disconnect

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Redirect,
    Extension, Json,
};
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::domain::oauth::{service, OAuthConnectionInfo, OAuthError, TokenResponse};
use crate::AppState;

// ============================================================================
// Error helpers
// ============================================================================

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub error: String,
    pub message: String,
}

impl ErrorBody {
    fn new(error: &str, message: &str) -> Self {
        Self {
            error: error.to_string(),
            message: message.to_string(),
        }
    }
}

fn oauth_error_response(e: OAuthError) -> (StatusCode, Json<ErrorBody>) {
    match e {
        OAuthError::NotFound => (
            StatusCode::NOT_FOUND,
            Json(ErrorBody::new("not_found", "No connection found")),
        ),
        OAuthError::UnsupportedProvider(p) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorBody::new(
                "unsupported_provider",
                &format!("Provider not supported: {}", p),
            )),
        ),
        OAuthError::TokenExchangeFailed(msg) => (
            StatusCode::BAD_GATEWAY,
            Json(ErrorBody::new("token_exchange_failed", &msg)),
        ),
        OAuthError::MissingEncryptionKey => {
            tracing::error!("OAUTH_ENCRYPTION_KEY not set");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody::new(
                    "configuration_error",
                    "Server misconfiguration",
                )),
            )
        }
        OAuthError::DuplicateConnection(msg) => (
            StatusCode::CONFLICT,
            Json(ErrorBody::new("duplicate_connection", &msg)),
        ),
        OAuthError::AlreadyDisconnected => (
            StatusCode::CONFLICT,
            Json(ErrorBody::new(
                "already_disconnected",
                "Connection is already disconnected",
            )),
        ),
        OAuthError::Database(e) => {
            tracing::error!("OAuth DB error: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody::new("database_error", "Internal database error")),
            )
        }
    }
}

fn extract_tenant(
    claims: &Option<Extension<VerifiedClaims>>,
) -> Result<String, (StatusCode, Json<ErrorBody>)> {
    match claims {
        Some(Extension(c)) => Ok(c.tenant_id.to_string()),
        None => Err((
            StatusCode::UNAUTHORIZED,
            Json(ErrorBody::new(
                "unauthorized",
                "Missing or invalid authentication",
            )),
        )),
    }
}

fn validate_provider(provider: &str) -> Result<(), (StatusCode, Json<ErrorBody>)> {
    match provider {
        "quickbooks" => Ok(()),
        _ => Err(oauth_error_response(OAuthError::UnsupportedProvider(
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
    fn from_env() -> Result<Self, (StatusCode, Json<ErrorBody>)> {
        let missing = |var: &str| {
            tracing::error!("{} env var not set", var);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody::new(
                    "configuration_error",
                    "Server misconfiguration",
                )),
            )
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

/// GET /api/integrations/oauth/connect/{provider}
///
/// Redirects the user to the provider's authorization page.
pub async fn connect(
    claims: Option<Extension<VerifiedClaims>>,
    Path(provider): Path<String>,
) -> Result<Redirect, (StatusCode, Json<ErrorBody>)> {
    let app_id = extract_tenant(&claims)?;
    validate_provider(&provider)?;

    let config = QboConfig::from_env()?;

    let scopes = "com.intuit.quickbooks.accounting";
    let auth_url = format!(
        "{}?client_id={}&response_type=code&scope={}&redirect_uri={}&state={}",
        config.auth_url,
        urlencoding::encode(&config.client_id),
        urlencoding::encode(scopes),
        urlencoding::encode(&config.redirect_uri),
        urlencoding::encode(&app_id),
    );

    Ok(Redirect::temporary(&auth_url))
}

/// GET /api/integrations/oauth/callback/{provider}
///
/// Handles the redirect from the provider after user authorization.
/// Exchanges the auth code for tokens and persists the connection.
pub async fn callback(
    State(state): State<Arc<AppState>>,
    Path(provider): Path<String>,
    Query(params): Query<OAuthCallbackQuery>,
) -> Result<(StatusCode, Json<OAuthConnectionInfo>), (StatusCode, Json<ErrorBody>)> {
    validate_provider(&provider)?;

    let config = QboConfig::from_env()?;

    // The state parameter carries the app_id set during connect
    let app_id = params.state.as_deref().unwrap_or("default");

    // Exchange authorization code for tokens
    let client = reqwest::Client::new();
    let resp = client
        .post(&config.token_url)
        .basic_auth(&config.client_id, Some(&config.client_secret))
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", &params.code),
            ("redirect_uri", &config.redirect_uri),
        ])
        .send()
        .await
        .map_err(|e| {
            oauth_error_response(OAuthError::TokenExchangeFailed(format!(
                "HTTP request failed: {}",
                e
            )))
        })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(oauth_error_response(OAuthError::TokenExchangeFailed(
            format!("Token exchange failed: HTTP {} — {}", status, body),
        )));
    }

    let tokens: TokenResponse = resp.json().await.map_err(|e| {
        oauth_error_response(OAuthError::TokenExchangeFailed(format!(
            "Failed to parse token response: {}",
            e
        )))
    })?;

    let scopes = "com.intuit.quickbooks.accounting";

    let connection = service::create_connection(
        &state.pool,
        app_id,
        &provider,
        &params.realm_id,
        scopes,
        &tokens,
    )
    .await
    .map_err(oauth_error_response)?;

    Ok((StatusCode::CREATED, Json(connection)))
}

/// GET /api/integrations/oauth/status/{provider}
///
/// Returns the connection status for the current tenant + provider.
pub async fn status(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(provider): Path<String>,
) -> Result<Json<OAuthConnectionInfo>, (StatusCode, Json<ErrorBody>)> {
    let app_id = extract_tenant(&claims)?;
    validate_provider(&provider)?;

    let info = service::get_connection_status(&state.pool, &app_id, &provider)
        .await
        .map_err(oauth_error_response)?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorBody::new("not_found", "No connection found")),
            )
        })?;

    Ok(Json(info))
}

/// POST /api/integrations/oauth/disconnect/{provider}
///
/// Marks the connection as disconnected.
pub async fn disconnect(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(provider): Path<String>,
) -> Result<Json<OAuthConnectionInfo>, (StatusCode, Json<ErrorBody>)> {
    let app_id = extract_tenant(&claims)?;
    validate_provider(&provider)?;

    let info = service::disconnect(&state.pool, &app_id, &provider)
        .await
        .map_err(oauth_error_response)?;

    Ok(Json(info))
}
