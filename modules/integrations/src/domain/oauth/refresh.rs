//! Background token refresh worker.
//!
//! Polls for connections where `access_token_expires_at < NOW() + 10 minutes`
//! and refreshes them proactively. Uses `SELECT ... FOR UPDATE SKIP LOCKED`
//! to prevent concurrent refresh of the same connection.
//!
//! Runs as a tokio background task with a configurable poll interval.

use chrono::{Duration, Utc};
use sqlx::PgPool;
use std::sync::Arc;
use tokio::sync::watch;

use super::service::encryption_key;
use super::TokenResponse;

/// Trait for HTTP token exchange — allows injecting test implementations.
#[async_trait::async_trait]
pub trait TokenRefresher: Send + Sync {
    async fn refresh_token(
        &self,
        provider: &str,
        refresh_token: &str,
    ) -> Result<TokenResponse, String>;
}

/// Production token refresher that calls provider token endpoints.
pub struct HttpTokenRefresher {
    pub client: reqwest::Client,
    pub qbo_client_id: String,
    pub qbo_client_secret: String,
    pub qbo_token_url: String,
}

#[async_trait::async_trait]
impl TokenRefresher for HttpTokenRefresher {
    async fn refresh_token(
        &self,
        provider: &str,
        refresh_token: &str,
    ) -> Result<TokenResponse, String> {
        match provider {
            "quickbooks" => {
                let resp = self
                    .client
                    .post(&self.qbo_token_url)
                    .basic_auth(&self.qbo_client_id, Some(&self.qbo_client_secret))
                    .form(&[
                        ("grant_type", "refresh_token"),
                        ("refresh_token", refresh_token),
                    ])
                    .send()
                    .await
                    .map_err(|e| format!("HTTP request failed: {}", e))?;

                if !resp.status().is_success() {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    return Err(format!(
                        "Token refresh failed: HTTP {} — {}",
                        status, body
                    ));
                }

                resp.json::<TokenResponse>()
                    .await
                    .map_err(|e| format!("Failed to parse token response: {}", e))
            }
            other => Err(format!("No refresh implementation for provider: {}", other)),
        }
    }
}

/// Row returned by the refresh query (includes decrypted refresh token).
#[derive(Debug, sqlx::FromRow)]
struct RefreshCandidate {
    id: uuid::Uuid,
    provider: String,
    refresh_token_plaintext: String,
}

/// Run one refresh tick: find expiring connections and refresh them.
///
/// Returns the number of connections successfully refreshed.
pub async fn refresh_tick(
    pool: &PgPool,
    refresher: &dyn TokenRefresher,
) -> Result<usize, sqlx::Error> {
    let key = match encryption_key() {
        Ok(k) => k,
        Err(_) => {
            tracing::warn!("OAUTH_ENCRYPTION_KEY not set — skipping refresh tick");
            return Ok(0);
        }
    };

    // Find connections where access token expires within 10 minutes.
    // FOR UPDATE SKIP LOCKED prevents concurrent refresh of the same row.
    let candidates = sqlx::query_as::<_, RefreshCandidate>(
        r#"
        SELECT id, provider,
               pgp_sym_decrypt(refresh_token, $1) AS refresh_token_plaintext
        FROM integrations_oauth_connections
        WHERE connection_status = 'connected'
          AND access_token_expires_at < NOW() + INTERVAL '10 minutes'
        FOR UPDATE SKIP LOCKED
        "#,
    )
    .bind(&key)
    .fetch_all(pool)
    .await?;

    let mut refreshed = 0;

    for candidate in &candidates {
        match refresher
            .refresh_token(&candidate.provider, &candidate.refresh_token_plaintext)
            .await
        {
            Ok(tokens) => {
                let now = Utc::now();
                let access_expires = now + Duration::seconds(tokens.expires_in);
                let refresh_expires = if tokens.x_refresh_token_expires_in > 0 {
                    now + Duration::seconds(tokens.x_refresh_token_expires_in)
                } else {
                    now + Duration::days(100)
                };

                let result = sqlx::query(
                    r#"
                    UPDATE integrations_oauth_connections
                    SET access_token = pgp_sym_encrypt($2, $3),
                        refresh_token = pgp_sym_encrypt($4, $3),
                        access_token_expires_at = $5,
                        refresh_token_expires_at = $6,
                        last_successful_refresh = $7,
                        updated_at = $7
                    WHERE id = $1
                    "#,
                )
                .bind(candidate.id)
                .bind(&tokens.access_token)
                .bind(&key)
                .bind(&tokens.refresh_token)
                .bind(access_expires)
                .bind(refresh_expires)
                .bind(now)
                .execute(pool)
                .await;

                match result {
                    Ok(_) => {
                        tracing::info!(
                            connection_id = %candidate.id,
                            provider = %candidate.provider,
                            "Token refreshed successfully"
                        );
                        refreshed += 1;
                    }
                    Err(e) => {
                        tracing::error!(
                            connection_id = %candidate.id,
                            error = %e,
                            "Failed to persist refreshed tokens"
                        );
                    }
                }
            }
            Err(err) => {
                tracing::error!(
                    connection_id = %candidate.id,
                    provider = %candidate.provider,
                    error = %err,
                    "Token refresh failed — marking needs_reauth"
                );

                let _ = sqlx::query(
                    r#"
                    UPDATE integrations_oauth_connections
                    SET connection_status = 'needs_reauth', updated_at = NOW()
                    WHERE id = $1
                    "#,
                )
                .bind(candidate.id)
                .execute(pool)
                .await;
            }
        }
    }

    Ok(refreshed)
}

/// Spawn the background refresh worker as a tokio task.
///
/// The worker runs `refresh_tick` every `poll_interval` until the shutdown
/// signal is received via the `shutdown_rx` channel.
pub fn spawn_refresh_worker(
    pool: PgPool,
    refresher: Arc<dyn TokenRefresher>,
    poll_interval: std::time::Duration,
    mut shutdown_rx: watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        tracing::info!(
            poll_interval_secs = poll_interval.as_secs(),
            "OAuth refresh worker started"
        );

        loop {
            tokio::select! {
                _ = tokio::time::sleep(poll_interval) => {
                    match refresh_tick(&pool, refresher.as_ref()).await {
                        Ok(n) if n > 0 => {
                            tracing::info!(refreshed = n, "Refresh tick completed");
                        }
                        Ok(_) => {} // nothing to refresh, stay quiet
                        Err(e) => {
                            tracing::error!(error = %e, "Refresh tick failed");
                        }
                    }
                }
                _ = shutdown_rx.changed() => {
                    tracing::info!("OAuth refresh worker shutting down");
                    break;
                }
            }
        }
    })
}
