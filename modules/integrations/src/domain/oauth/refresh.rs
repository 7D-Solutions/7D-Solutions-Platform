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

use super::repo;
use super::service::encryption_key;
use super::TokenResponse;
use crate::domain::sync::health;

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
                    return Err(format!("Token refresh failed: HTTP {} — {}", status, body));
                }

                resp.json::<TokenResponse>()
                    .await
                    .map_err(|e| format!("Failed to parse token response: {}", e))
            }
            "ups" => {
                let client_id = std::env::var("UPS_CLIENT_ID")
                    .map_err(|_| "UPS_CLIENT_ID not configured".to_string())?;
                let client_secret = std::env::var("UPS_CLIENT_SECRET")
                    .map_err(|_| "UPS_CLIENT_SECRET not configured".to_string())?;
                let token_url = std::env::var("UPS_TOKEN_URL").unwrap_or_else(|_| {
                    "https://onlinetools.ups.com/security/v1/oauth/refresh".to_string()
                });

                let resp = self
                    .client
                    .post(&token_url)
                    .basic_auth(&client_id, Some(&client_secret))
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
                    return Err(format!("Token refresh failed: HTTP {} — {}", status, body));
                }

                resp.json::<TokenResponse>()
                    .await
                    .map_err(|e| format!("Failed to parse token response: {}", e))
            }
            "fedex" => {
                let client_id = std::env::var("FEDEX_CLIENT_ID")
                    .map_err(|_| "FEDEX_CLIENT_ID not configured".to_string())?;
                let client_secret = std::env::var("FEDEX_CLIENT_SECRET")
                    .map_err(|_| "FEDEX_CLIENT_SECRET not configured".to_string())?;
                let token_url = std::env::var("FEDEX_TOKEN_URL").unwrap_or_else(|_| {
                    "https://apis.fedex.com/oauth/token".to_string()
                });

                #[derive(serde::Deserialize)]
                struct FedexTokenResponse {
                    access_token: String,
                    expires_in: i64,
                }

                let resp = self
                    .client
                    .post(&token_url)
                    .form(&[
                        ("grant_type", "client_credentials"),
                        ("client_id", client_id.as_str()),
                        ("client_secret", client_secret.as_str()),
                        ("scope", "oob"),
                    ])
                    .send()
                    .await
                    .map_err(|e| format!("HTTP request failed: {}", e))?;

                if !resp.status().is_success() {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    return Err(format!("Token refresh failed: HTTP {} — {}", status, body));
                }

                let fedex = resp
                    .json::<FedexTokenResponse>()
                    .await
                    .map_err(|e| format!("Failed to parse FedEx token response: {}", e))?;

                Ok(TokenResponse {
                    access_token: fedex.access_token,
                    refresh_token: String::new(),
                    expires_in: fedex.expires_in,
                    x_refresh_token_expires_in: 0,
                })
            }
            other => Err(format!("No refresh implementation for provider: {}", other)),
        }
    }
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

    let candidates = repo::get_refresh_candidates(pool, &key).await?;

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

                let result = repo::update_tokens(
                    pool,
                    candidate.id,
                    &tokens.access_token,
                    &key,
                    &tokens.refresh_token,
                    access_expires,
                    refresh_expires,
                    now,
                )
                .await;

                match result {
                    Ok(_) => {
                        tracing::info!(
                            connection_id = %candidate.id,
                            provider = %candidate.provider,
                            "Token refreshed successfully"
                        );
                        if let Err(e) = health::upsert_job_success(
                            pool,
                            &candidate.app_id,
                            &candidate.provider,
                            "token_refresh",
                        )
                        .await
                        {
                            tracing::warn!(error = %e, "Failed to record token_refresh health");
                        }
                        refreshed += 1;
                    }
                    Err(e) => {
                        tracing::error!(
                            connection_id = %candidate.id,
                            error = %e,
                            "Failed to persist refreshed tokens"
                        );
                        if let Err(he) = health::upsert_job_failure(
                            pool,
                            &candidate.app_id,
                            &candidate.provider,
                            "token_refresh",
                            &e.to_string(),
                        )
                        .await
                        {
                            tracing::warn!(error = %he, "Failed to record token_refresh health");
                        }
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

                let _ = repo::mark_needs_reauth(pool, candidate.id).await;
                if let Err(he) = health::upsert_job_failure(
                    pool,
                    &candidate.app_id,
                    &candidate.provider,
                    "token_refresh",
                    &err,
                )
                .await
                {
                    tracing::warn!(error = %he, "Failed to record token_refresh health");
                }
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
