//! OAuth connection service — CRUD for `integrations_oauth_connections`.
//!
//! All token values are encrypted at rest via pgcrypto `pgp_sym_encrypt` with AES-256.
//! The encryption key is read from `OAUTH_ENCRYPTION_KEY` env var.

use chrono::{Duration, Utc};
use sqlx::PgPool;

use super::repo;
use super::{OAuthConnectionInfo, OAuthError, TokenResponse};

/// Read the encryption key from environment. Fails loudly if missing.
pub fn encryption_key() -> Result<String, OAuthError> {
    std::env::var("OAUTH_ENCRYPTION_KEY").map_err(|_| OAuthError::MissingEncryptionKey)
}

// ============================================================================
// Reads
// ============================================================================

/// Get connection status for a tenant + provider (safe — no token data).
pub async fn get_connection_status(
    pool: &PgPool,
    app_id: &str,
    provider: &str,
) -> Result<Option<OAuthConnectionInfo>, OAuthError> {
    let row = repo::get_connection(pool, app_id, provider).await?;
    Ok(row.map(OAuthConnectionInfo::from))
}

/// Decrypt and return the current access token for a connection.
/// Used by the QBO REST client — never exposed via HTTP.
pub async fn get_access_token(
    pool: &PgPool,
    app_id: &str,
    provider: &str,
) -> Result<String, OAuthError> {
    let key = encryption_key()?;
    let row = repo::get_decrypted_access_token(pool, app_id, provider, &key).await?;
    row.map(|r| r.0).ok_or(OAuthError::NotFound)
}

// ============================================================================
// Writes
// ============================================================================

/// Create a new OAuth connection from a token exchange response.
/// Called from the OAuth callback handler after exchanging the auth code.
pub async fn create_connection(
    pool: &PgPool,
    app_id: &str,
    provider: &str,
    realm_id: &str,
    scopes: &str,
    tokens: &TokenResponse,
) -> Result<OAuthConnectionInfo, OAuthError> {
    let key = encryption_key()?;
    let now = Utc::now();
    let access_expires = now + Duration::seconds(tokens.expires_in);
    let refresh_expires = if tokens.x_refresh_token_expires_in > 0 {
        now + Duration::seconds(tokens.x_refresh_token_expires_in)
    } else {
        // Default to 100 days if provider doesn't specify
        now + Duration::days(100)
    };

    let row = repo::upsert_connection(
        pool,
        app_id,
        provider,
        realm_id,
        &tokens.access_token,
        &key,
        &tokens.refresh_token,
        access_expires,
        refresh_expires,
        scopes,
    )
    .await
    .map_err(|e| {
        let msg = e.to_string();
        if msg.contains("integrations_oauth_connections_provider_realm_connected") {
            OAuthError::DuplicateConnection(format!(
                "Provider '{}' realm '{}' is already connected to another tenant",
                provider, realm_id
            ))
        } else {
            OAuthError::Database(e)
        }
    })?;

    Ok(OAuthConnectionInfo::from(row))
}

/// Import pre-existing OAuth tokens directly (dev/CI seeding path).
///
/// Uses the same `pgp_sym_encrypt` path as `create_connection`, ensuring
/// imported tokens are encrypted identically to callback-issued tokens.
/// The caller supplies raw expiry durations; the function computes absolute
/// timestamps from `Utc::now()` identically to `create_connection`.
pub async fn import_connection(
    pool: &PgPool,
    app_id: &str,
    provider: &str,
    realm_id: &str,
    access_token: &str,
    refresh_token: &str,
    expires_in: i64,
    refresh_token_expires_in: i64,
    scopes: &str,
) -> Result<OAuthConnectionInfo, OAuthError> {
    let key = encryption_key()?;
    let now = Utc::now();
    let access_expires = now + Duration::seconds(expires_in);
    let refresh_expires = if refresh_token_expires_in > 0 {
        now + Duration::seconds(refresh_token_expires_in)
    } else {
        now + Duration::days(100)
    };

    let row = repo::upsert_connection(
        pool,
        app_id,
        provider,
        realm_id,
        access_token,
        &key,
        refresh_token,
        access_expires,
        refresh_expires,
        scopes,
    )
    .await
    .map_err(|e| {
        let msg = e.to_string();
        if msg.contains("integrations_oauth_connections_provider_realm_connected") {
            OAuthError::DuplicateConnection(format!(
                "Provider '{}' realm '{}' is already connected to another tenant",
                provider, realm_id
            ))
        } else {
            OAuthError::Database(e)
        }
    })?;

    Ok(OAuthConnectionInfo::from(row))
}

/// Mark a connection as disconnected.
pub async fn disconnect(
    pool: &PgPool,
    app_id: &str,
    provider: &str,
) -> Result<OAuthConnectionInfo, OAuthError> {
    let row = repo::set_disconnected(pool, app_id, provider)
        .await?
        .ok_or(OAuthError::NotFound)?;
    Ok(OAuthConnectionInfo::from(row))
}
