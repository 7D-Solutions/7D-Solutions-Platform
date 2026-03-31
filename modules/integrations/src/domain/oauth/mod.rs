//! OAuth connection management — token storage, refresh, and lifecycle.
//!
//! Owns the `integrations_oauth_connections` table and provides:
//! - Connection CRUD (create from callback, status query, disconnect)
//! - Background token refresh worker (proactive, prevents expiry)
//!
//! Tokens are encrypted at rest via pgcrypto `pgp_sym_encrypt`.

pub mod refresh;
pub mod service;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use utoipa::ToSchema;

// ============================================================================
// Error type
// ============================================================================

#[derive(Debug, Error)]
pub enum OAuthError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("Connection not found")]
    NotFound,
    #[error("Provider not supported: {0}")]
    UnsupportedProvider(String),
    #[error("Token exchange failed: {0}")]
    TokenExchangeFailed(String),
    #[error("Missing encryption key: OAUTH_ENCRYPTION_KEY env var not set")]
    MissingEncryptionKey,
    #[error("Duplicate connection: {0}")]
    DuplicateConnection(String),
    #[error("Connection already disconnected")]
    AlreadyDisconnected,
}

// ============================================================================
// Connection status
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionStatus {
    Connected,
    Disconnected,
    NeedsReauth,
}

impl ConnectionStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Connected => "connected",
            Self::Disconnected => "disconnected",
            Self::NeedsReauth => "needs_reauth",
        }
    }
}

// ============================================================================
// DB row model (no plaintext tokens — encrypted BYTEA)
// ============================================================================

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct OAuthConnectionRow {
    pub id: uuid::Uuid,
    pub app_id: String,
    pub provider: String,
    pub realm_id: String,
    pub access_token_expires_at: DateTime<Utc>,
    pub refresh_token_expires_at: DateTime<Utc>,
    pub scopes_granted: String,
    pub connection_status: String,
    pub last_successful_refresh: Option<DateTime<Utc>>,
    pub cdc_watermark: Option<DateTime<Utc>>,
    pub full_resync_required: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ============================================================================
// API response (safe — never exposes tokens)
// ============================================================================

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct OAuthConnectionInfo {
    pub id: uuid::Uuid,
    pub app_id: String,
    pub provider: String,
    pub realm_id: String,
    pub scopes_granted: String,
    pub connection_status: String,
    pub access_token_expires_at: DateTime<Utc>,
    pub refresh_token_expires_at: DateTime<Utc>,
    pub last_successful_refresh: Option<DateTime<Utc>>,
    pub cdc_watermark: Option<DateTime<Utc>>,
    pub full_resync_required: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<OAuthConnectionRow> for OAuthConnectionInfo {
    fn from(row: OAuthConnectionRow) -> Self {
        Self {
            id: row.id,
            app_id: row.app_id,
            provider: row.provider,
            realm_id: row.realm_id,
            scopes_granted: row.scopes_granted,
            connection_status: row.connection_status,
            access_token_expires_at: row.access_token_expires_at,
            refresh_token_expires_at: row.refresh_token_expires_at,
            last_successful_refresh: row.last_successful_refresh,
            cdc_watermark: row.cdc_watermark,
            full_resync_required: row.full_resync_required,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

// ============================================================================
// Token exchange response (from provider)
// ============================================================================

#[derive(Debug, Clone, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub refresh_token: String,
    /// Seconds until access token expires.
    pub expires_in: i64,
    /// Seconds until refresh token expires (Intuit-specific).
    #[serde(default)]
    pub x_refresh_token_expires_in: i64,
}
