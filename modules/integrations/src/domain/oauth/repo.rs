//! Repository layer for OAuth connection persistence.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use super::OAuthConnectionRow;

pub async fn get_connection(
    pool: &PgPool,
    app_id: &str,
    provider: &str,
) -> Result<Option<OAuthConnectionRow>, sqlx::Error> {
    sqlx::query_as::<_, OAuthConnectionRow>(
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
    .await
}

pub async fn get_decrypted_access_token(
    pool: &PgPool,
    app_id: &str,
    provider: &str,
    encryption_key: &str,
) -> Result<Option<(String,)>, sqlx::Error> {
    sqlx::query_as(
        r#"
        SELECT pgp_sym_decrypt(access_token, $3, 'cipher-algo=aes256')
        FROM integrations_oauth_connections
        WHERE app_id = $1 AND provider = $2
          AND connection_status = 'connected'
        "#,
    )
    .bind(app_id)
    .bind(provider)
    .bind(encryption_key)
    .fetch_optional(pool)
    .await
}

pub async fn upsert_connection(
    pool: &PgPool,
    app_id: &str,
    provider: &str,
    realm_id: &str,
    access_token: &str,
    encryption_key: &str,
    refresh_token: &str,
    access_expires: DateTime<Utc>,
    refresh_expires: DateTime<Utc>,
    scopes: &str,
) -> Result<OAuthConnectionRow, sqlx::Error> {
    sqlx::query_as::<_, OAuthConnectionRow>(
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
            pgp_sym_encrypt($4, $5, 'cipher-algo=aes256'), pgp_sym_encrypt($6, $5, 'cipher-algo=aes256'),
            $7, $8,
            $9, 'connected',
            NOW(), NOW()
        )
        ON CONFLICT (app_id, provider) DO UPDATE SET
            realm_id                 = EXCLUDED.realm_id,
            access_token             = EXCLUDED.access_token,
            refresh_token            = EXCLUDED.refresh_token,
            access_token_expires_at  = EXCLUDED.access_token_expires_at,
            refresh_token_expires_at = EXCLUDED.refresh_token_expires_at,
            scopes_granted           = EXCLUDED.scopes_granted,
            connection_status        = 'connected',
            updated_at               = NOW()
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
    .bind(access_token)
    .bind(encryption_key)
    .bind(refresh_token)
    .bind(access_expires)
    .bind(refresh_expires)
    .bind(scopes)
    .fetch_one(pool)
    .await
}

pub async fn set_disconnected(
    pool: &PgPool,
    app_id: &str,
    provider: &str,
) -> Result<Option<OAuthConnectionRow>, sqlx::Error> {
    sqlx::query_as::<_, OAuthConnectionRow>(
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
    .await
}

// -- Refresh worker queries --

#[derive(Debug, sqlx::FromRow)]
pub struct RefreshCandidate {
    pub id: Uuid,
    pub app_id: String,
    pub provider: String,
    pub refresh_token_plaintext: String,
}

pub async fn get_refresh_candidates(
    pool: &PgPool,
    encryption_key: &str,
) -> Result<Vec<RefreshCandidate>, sqlx::Error> {
    sqlx::query_as::<_, RefreshCandidate>(
        r#"
        SELECT id, app_id, provider,
               pgp_sym_decrypt(refresh_token, $1, 'cipher-algo=aes256') AS refresh_token_plaintext
        FROM integrations_oauth_connections
        WHERE connection_status = 'connected'
          AND access_token_expires_at < NOW() + INTERVAL '10 minutes'
        FOR UPDATE SKIP LOCKED
        "#,
    )
    .bind(encryption_key)
    .fetch_all(pool)
    .await
}

pub async fn update_tokens(
    pool: &PgPool,
    connection_id: Uuid,
    access_token: &str,
    encryption_key: &str,
    refresh_token: &str,
    access_expires: DateTime<Utc>,
    refresh_expires: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE integrations_oauth_connections
        SET access_token = pgp_sym_encrypt($2, $3, 'cipher-algo=aes256'),
            refresh_token = pgp_sym_encrypt($4, $3, 'cipher-algo=aes256'),
            access_token_expires_at = $5,
            refresh_token_expires_at = $6,
            last_successful_refresh = $7,
            updated_at = $7
        WHERE id = $1
        "#,
    )
    .bind(connection_id)
    .bind(access_token)
    .bind(encryption_key)
    .bind(refresh_token)
    .bind(access_expires)
    .bind(refresh_expires)
    .bind(now)
    .execute(pool)
    .await
}

pub async fn mark_needs_reauth(
    pool: &PgPool,
    connection_id: Uuid,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE integrations_oauth_connections
        SET connection_status = 'needs_reauth', updated_at = NOW()
        WHERE id = $1
        "#,
    )
    .bind(connection_id)
    .execute(pool)
    .await
}
