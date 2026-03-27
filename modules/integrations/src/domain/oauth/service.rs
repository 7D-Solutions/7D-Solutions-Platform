//! OAuth connection service — CRUD for `integrations_oauth_connections`.
//!
//! All token values are encrypted at rest via pgcrypto `pgp_sym_encrypt`.
//! The encryption key is read from `OAUTH_ENCRYPTION_KEY` env var.

use chrono::{Duration, Utc};
use sqlx::PgPool;

use super::{OAuthConnectionInfo, OAuthConnectionRow, OAuthError, TokenResponse};

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
    let row = sqlx::query_as::<_, OAuthConnectionRow>(
        r#"
        SELECT id, app_id, provider, realm_id,
               access_token_expires_at, refresh_token_expires_at,
               scopes_granted, connection_status,
               last_successful_refresh, cdc_watermark, full_resync_required,
               created_at, updated_at
        FROM integrations_oauth_connections
        WHERE app_id = $1 AND provider = $2
        "#,
    )
    .bind(app_id)
    .bind(provider)
    .fetch_optional(pool)
    .await?;

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

    let row: Option<(String,)> = sqlx::query_as(
        r#"
        SELECT pgp_sym_decrypt(access_token, $3)
        FROM integrations_oauth_connections
        WHERE app_id = $1 AND provider = $2
          AND connection_status = 'connected'
        "#,
    )
    .bind(app_id)
    .bind(provider)
    .bind(&key)
    .fetch_optional(pool)
    .await?;

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

    let row = sqlx::query_as::<_, OAuthConnectionRow>(
        r#"
        INSERT INTO integrations_oauth_connections (
            app_id, provider, realm_id,
            access_token, refresh_token,
            access_token_expires_at, refresh_token_expires_at,
            scopes_granted, connection_status,
            created_at, updated_at
        )
        VALUES (
            $1, $2, $3,
            pgp_sym_encrypt($4, $5), pgp_sym_encrypt($6, $5),
            $7, $8,
            $9, 'connected',
            NOW(), NOW()
        )
        RETURNING id, app_id, provider, realm_id,
                  access_token_expires_at, refresh_token_expires_at,
                  scopes_granted, connection_status,
                  last_successful_refresh, cdc_watermark, full_resync_required,
                  created_at, updated_at
        "#,
    )
    .bind(app_id)
    .bind(provider)
    .bind(realm_id)
    .bind(&tokens.access_token)
    .bind(&key)
    .bind(&tokens.refresh_token)
    .bind(access_expires)
    .bind(refresh_expires)
    .bind(scopes)
    .fetch_one(pool)
    .await
    .map_err(|e| {
        let msg = e.to_string();
        if msg.contains("integrations_oauth_connections_provider_realm_unique") {
            OAuthError::DuplicateConnection(format!(
                "Provider '{}' realm '{}' is already connected to another tenant",
                provider, realm_id
            ))
        } else if msg.contains("integrations_oauth_connections_app_provider_unique") {
            OAuthError::DuplicateConnection(format!(
                "Tenant already has an active '{}' connection",
                provider
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
    let row = sqlx::query_as::<_, OAuthConnectionRow>(
        r#"
        UPDATE integrations_oauth_connections
        SET connection_status = 'disconnected', updated_at = NOW()
        WHERE app_id = $1 AND provider = $2
          AND connection_status != 'disconnected'
        RETURNING id, app_id, provider, realm_id,
                  access_token_expires_at, refresh_token_expires_at,
                  scopes_granted, connection_status,
                  last_successful_refresh, cdc_watermark, full_resync_required,
                  created_at, updated_at
        "#,
    )
    .bind(app_id)
    .bind(provider)
    .fetch_optional(pool)
    .await?
    .ok_or(OAuthError::NotFound)?;

    Ok(OAuthConnectionInfo::from(row))
}
