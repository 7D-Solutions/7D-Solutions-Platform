//! HTTP handlers for OAuth connection lifecycle.
//!
//! Routes:
//!   GET  /api/integrations/oauth/connect/{provider}     — redirect to provider auth URL (307)
//!   GET  /api/integrations/oauth/callback/{provider}    — handle provider redirect (always 302)
//!   GET  /api/integrations/oauth/status/{provider}      — connection status
//!   POST /api/integrations/oauth/disconnect/{provider}  — disconnect
//!   POST /api/integrations/oauth/import                 — seed tokens directly (admin + env gate)

use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Redirect, Response},
    Extension, Json,
};
use base64::Engine as _;
use hmac::{Hmac, Mac};
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::sync::Arc;

use crate::domain::oauth::{service, OAuthConnectionInfo, OAuthError, TokenResponse};
use crate::AppState;
use platform_sdk::extract_tenant;

type HmacSha256 = Hmac<Sha256>;

// ============================================================================
// Signed-state helpers
// ============================================================================

/// State payload carried through the OAuth redirect roundtrip.
#[derive(Debug, Serialize, Deserialize)]
pub struct OAuthStatePayload {
    pub app_id: String,
    pub return_url: String,
    pub nonce: String,
}

fn new_nonce() -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(uuid::Uuid::new_v4().as_bytes())
}

fn state_secret() -> Result<Vec<u8>, String> {
    let raw = std::env::var("OAUTH_STATE_SECRET")
        .map_err(|_| "OAUTH_STATE_SECRET not set".to_string())?;
    if raw.len() < 32 {
        return Err("OAUTH_STATE_SECRET too short".to_string());
    }
    Ok(raw.into_bytes())
}

/// Encode payload as `base64url(json).base64url(hmac_sha256)`.
pub fn encode_state(payload: &OAuthStatePayload) -> Result<String, String> {
    let json = serde_json::to_string(payload).map_err(|e| e.to_string())?;
    let encoded_json =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json.as_bytes());

    let secret = state_secret()?;
    let mut mac =
        HmacSha256::new_from_slice(&secret).map_err(|e| format!("HMAC init: {e}"))?;
    mac.update(encoded_json.as_bytes());
    let sig = mac.finalize().into_bytes();
    let encoded_sig = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sig.as_slice());

    Ok(format!("{}.{}", encoded_json, encoded_sig))
}

/// Decode and HMAC-verify a state string produced by `encode_state`.
pub fn decode_state(state: &str) -> Result<OAuthStatePayload, String> {
    let dot = state.rfind('.').ok_or("invalid state format: no dot separator")?;
    let encoded_json = &state[..dot];
    let encoded_sig = &state[dot + 1..];

    let sig_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded_sig)
        .map_err(|_| "invalid signature encoding")?;

    let secret = state_secret()?;
    let mut mac =
        HmacSha256::new_from_slice(&secret).map_err(|e| format!("HMAC init: {e}"))?;
    mac.update(encoded_json.as_bytes());
    mac.verify_slice(&sig_bytes)
        .map_err(|_| "HMAC verification failed")?;

    let json_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded_json)
        .map_err(|_| "invalid payload encoding")?;
    let payload: OAuthStatePayload =
        serde_json::from_slice(&json_bytes).map_err(|_| "invalid payload JSON")?;

    Ok(payload)
}

/// Append a query param to a URL, using '&' or '?' as appropriate.
pub fn append_query(base: &str, param: &str) -> String {
    if base.contains('?') {
        format!("{}&{}", base, param)
    } else {
        format!("{}?{}", base, param)
    }
}

/// Extract scheme://host[:port] from a URL string.
fn extract_origin(url: &str) -> Option<String> {
    let scheme_end = url.find("://")?;
    let scheme = &url[..scheme_end];
    let after_scheme = &url[scheme_end + 3..];
    let authority_end = after_scheme
        .find(|c: char| c == '/' || c == '?')
        .unwrap_or(after_scheme.len());
    Some(format!("{}://{}", scheme, &after_scheme[..authority_end]))
}

/// Return true if the origin of `url` exactly matches any comma-separated entry in
/// `allowed_origins` (case-insensitive scheme+host+port comparison).
fn is_origin_allowed(url: &str, allowed_origins: &str) -> bool {
    let origin = match extract_origin(url) {
        Some(o) => o.to_lowercase(),
        None => return false,
    };
    for entry in allowed_origins.split(',') {
        let entry = entry.trim().to_lowercase();
        if !entry.is_empty() && origin == entry {
            return true;
        }
    }
    false
}

fn default_return_url() -> String {
    std::env::var("OAUTH_DEFAULT_RETURN_URL").unwrap_or_default()
}

/// Build a true 302 Found redirect response (Intuit go-live requirement).
fn redirect_302(url: &str) -> Response {
    Response::builder()
        .status(StatusCode::FOUND)
        .header(header::LOCATION, url)
        .body(Body::empty())
        .expect("valid redirect response")
}

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
        "quickbooks" | "ups" | "fedex" => Ok(()),
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

struct UpsConfig {
    client_id: String,
    client_secret: String,
    redirect_uri: String,
    auth_url: String,
    token_url: String,
}

impl UpsConfig {
    fn from_env() -> Self {
        Self {
            client_id: std::env::var("UPS_CLIENT_ID").unwrap_or_default(),
            client_secret: std::env::var("UPS_CLIENT_SECRET").unwrap_or_default(),
            redirect_uri: std::env::var("UPS_REDIRECT_URI").unwrap_or_default(),
            auth_url: std::env::var("UPS_AUTH_URL").unwrap_or_else(|_| {
                "https://onlinetools.ups.com/security/v1/oauth/authorize".to_string()
            }),
            token_url: std::env::var("UPS_TOKEN_URL").unwrap_or_else(|_| {
                "https://onlinetools.ups.com/security/v1/oauth/token".to_string()
            }),
        }
    }
}

struct FedExConfig {
    client_id: String,
    client_secret: String,
    redirect_uri: String,
    auth_url: String,
    token_url: String,
}

impl FedExConfig {
    fn from_env() -> Self {
        Self {
            client_id: std::env::var("FEDEX_CLIENT_ID").unwrap_or_default(),
            client_secret: std::env::var("FEDEX_CLIENT_SECRET").unwrap_or_default(),
            redirect_uri: std::env::var("FEDEX_REDIRECT_URI").unwrap_or_default(),
            auth_url: std::env::var("FEDEX_AUTH_URL")
                .unwrap_or_else(|_| "https://apis.fedex.com/oauth/authorize".to_string()),
            token_url: std::env::var("FEDEX_TOKEN_URL")
                .unwrap_or_else(|_| "https://apis.fedex.com/oauth/token".to_string()),
        }
    }
}

// ============================================================================
// Query params
// ============================================================================

#[derive(Debug, Deserialize, Default)]
pub struct ConnectQuery {
    pub return_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct OAuthCallbackQuery {
    pub code: String,
    #[serde(rename = "realmId", default)]
    pub realm_id: Option<String>,
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
        (status = 422, description = "Unsupported provider or return_url origin not allowed"),
    ),
    security(("bearer" = [])),
    tag = "OAuth"
)]
/// GET /api/integrations/oauth/connect/{provider}
///
/// Redirects the user to the provider's authorization page (307).
/// Optional `return_url` query param controls post-callback destination.
/// The origin of return_url must be in OAUTH_ALLOWED_RETURN_ORIGINS or 422 is returned.
pub async fn connect(
    claims: Option<Extension<VerifiedClaims>>,
    Path(provider): Path<String>,
    Query(query): Query<ConnectQuery>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    if let Err(e) = validate_provider(&provider) {
        return e.into_response();
    }

    let allowed_origins = std::env::var("OAUTH_ALLOWED_RETURN_ORIGINS").unwrap_or_default();
    let return_url = match query.return_url {
        Some(ru) if !ru.is_empty() => {
            if !is_origin_allowed(&ru, &allowed_origins) {
                return ApiError::new(
                    422,
                    "origin_not_allowed",
                    "return_url origin is not in the allowed return origins list",
                )
                .into_response();
            }
            ru
        }
        _ => default_return_url(),
    };

    let payload = OAuthStatePayload {
        app_id: app_id.clone(),
        return_url,
        nonce: new_nonce(),
    };
    let state = match encode_state(&payload) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "Failed to encode OAuth state");
            return ApiError::internal("Failed to build OAuth state").into_response();
        }
    };

    let auth_url = match provider.as_str() {
        "quickbooks" => {
            let config = match QboConfig::from_env() {
                Ok(c) => c,
                Err(e) => return e.into_response(),
            };
            format!(
                "{}?client_id={}&response_type=code&scope={}&redirect_uri={}&state={}",
                config.auth_url,
                urlencoding::encode(&config.client_id),
                urlencoding::encode("com.intuit.quickbooks.accounting"),
                urlencoding::encode(&config.redirect_uri),
                urlencoding::encode(&state),
            )
        }
        "ups" => {
            let config = UpsConfig::from_env();
            format!(
                "{}?client_id={}&response_type=code&scope={}&redirect_uri={}&state={}",
                config.auth_url,
                urlencoding::encode(&config.client_id),
                urlencoding::encode("shipping"),
                urlencoding::encode(&config.redirect_uri),
                urlencoding::encode(&state),
            )
        }
        "fedex" => {
            let config = FedExConfig::from_env();
            format!(
                "{}?client_id={}&response_type=code&scope={}&redirect_uri={}&state={}",
                config.auth_url,
                urlencoding::encode(&config.client_id),
                urlencoding::encode("shipping"),
                urlencoding::encode(&config.redirect_uri),
                urlencoding::encode(&state),
            )
        }
        _ => unreachable!("validate_provider already checked"),
    };

    Redirect::temporary(&auth_url).into_response()
}

#[utoipa::path(
    get,
    path = "/api/integrations/oauth/callback/{provider}",
    params(
        ("provider" = String, Path, description = "OAuth provider"),
        ("code" = String, Query, description = "Authorization code"),
        ("realmId" = String, Query, description = "QBO realm ID"),
        ("state" = Option<String>, Query, description = "HMAC-signed state payload"),
    ),
    responses(
        (status = 302, description = "Redirect to return_url with connected=1 or error=<reason>"),
    ),
    tag = "OAuth"
)]
/// GET /api/integrations/oauth/callback/{provider}
///
/// Handles the redirect from the provider after user authorization.
/// Always returns 302 — never a JSON body (Intuit go-live security requirement).
/// The return destination is extracted from the HMAC-verified state payload.
pub async fn callback(
    State(state): State<Arc<AppState>>,
    Path(provider): Path<String>,
    Query(params): Query<OAuthCallbackQuery>,
) -> Response {
    // Decode and HMAC-verify state before doing anything else (CSRF guard).
    let raw_state = match params.state.as_deref() {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => {
            tracing::warn!("OAuth callback: missing or empty state — possible CSRF");
            return redirect_302(&append_query(&default_return_url(), "error=invalid_state"));
        }
    };

    let decoded_state = match decode_state(&raw_state) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, "OAuth callback: state decode/HMAC failed — possible CSRF or replay");
            return redirect_302(&append_query(&default_return_url(), "error=invalid_state"));
        }
    };

    let app_id = decoded_state.app_id.clone();
    let return_url = decoded_state.return_url.clone();

    if validate_provider(&provider).is_err() {
        return redirect_302(&append_query(&return_url, "error=invalid_provider"));
    }

    let (token_url, client_id, client_secret, redirect_uri, realm_id, scopes) =
        match provider.as_str() {
            "quickbooks" => {
                let realm_id = match params.realm_id.as_deref() {
                    Some(r) if !r.is_empty() => r.to_string(),
                    _ => {
                        return redirect_302(&append_query(&return_url, "error=missing_realm_id"));
                    }
                };
                let config = match QboConfig::from_env() {
                    Ok(c) => c,
                    Err(_) => {
                        return redirect_302(&append_query(
                            &return_url,
                            "error=server_misconfiguration",
                        ));
                    }
                };
                (
                    config.token_url,
                    config.client_id,
                    config.client_secret,
                    config.redirect_uri,
                    realm_id,
                    "com.intuit.quickbooks.accounting".to_string(),
                )
            }
            "ups" => {
                let config = UpsConfig::from_env();
                (
                    config.token_url,
                    config.client_id,
                    config.client_secret,
                    config.redirect_uri,
                    String::new(),
                    "shipping".to_string(),
                )
            }
            "fedex" => {
                let config = FedExConfig::from_env();
                (
                    config.token_url,
                    config.client_id,
                    config.client_secret,
                    config.redirect_uri,
                    String::new(),
                    "shipping".to_string(),
                )
            }
            _ => unreachable!("validate_provider already checked"),
        };

    let client = reqwest::Client::new();
    let resp = match client
        .post(&token_url)
        .basic_auth(&client_id, Some(&client_secret))
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", params.code.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
        ])
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "OAuth token exchange HTTP error");
            return redirect_302(&append_query(&return_url, "error=token_exchange_failed"));
        }
    };

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        tracing::warn!(http_status = %status, body = %body, "OAuth token exchange non-2xx");
        return redirect_302(&append_query(&return_url, "error=token_exchange_failed"));
    }

    let tokens: TokenResponse = match resp.json().await {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(error = %e, "OAuth token response parse error");
            return redirect_302(&append_query(&return_url, "error=token_exchange_failed"));
        }
    };

    match service::create_connection(
        &state.pool,
        &app_id,
        &provider,
        &realm_id,
        &scopes,
        &tokens,
    )
    .await
    {
        Ok(_) => redirect_302(&append_query(&return_url, "connected=1")),
        Err(e) => {
            tracing::warn!(error = %e, "OAuth create_connection failed");
            redirect_302(&append_query(&return_url, "error=db_write_failed"))
        }
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

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ImportTokensRequest {
    pub provider: String,
    pub realm_id: String,
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: i64,
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
