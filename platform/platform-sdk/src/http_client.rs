//! Typed HTTP client for calling platform services with auto-injected headers and retry.

use reqwest::{Client, Response, StatusCode};
use serde::Serialize;
use std::time::Duration;
use uuid::Uuid;

use chrono::Utc;
use security::claims::VerifiedClaims;

/// Timeout configuration for outbound HTTP requests.
#[derive(Debug, Clone)]
pub struct TimeoutConfig {
    /// Maximum time for the entire request (default: 30s).
    pub request_timeout: Duration,
    /// Maximum time to establish a connection (default: 5s).
    pub connect_timeout: Duration,
}

impl Default for TimeoutConfig {
    fn default() -> Self {
        Self {
            request_timeout: Duration::from_secs(30),
            connect_timeout: Duration::from_secs(5),
        }
    }
}

/// HTTP client that injects platform headers and retries on 429/503.
///
/// ```rust,ignore
/// let party = PlatformClient::new(env::var("PARTY_BASE_URL")?);
/// let resp = party.get("/api/parties/123", &claims).await?;
/// ```
#[derive(Clone)]
pub struct PlatformClient {
    client: Client,
    base_url: String,
    bearer_token: Option<String>,
}

impl std::fmt::Debug for PlatformClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlatformClient")
            .field("base_url", &self.base_url)
            .field("bearer_token", &self.bearer_token.as_ref().map(|_| "***"))
            .finish()
    }
}

const MAX_RETRIES: u32 = 3;
const INITIAL_BACKOFF_MS: u64 = 100;

impl PlatformClient {
    pub fn new(base_url: String) -> Self {
        Self::with_timeout(base_url, TimeoutConfig::default())
    }

    /// Create a client with custom timeout configuration.
    pub fn with_timeout(base_url: String, timeout: TimeoutConfig) -> Self {
        let client = Client::builder()
            .timeout(timeout.request_timeout)
            .connect_timeout(timeout.connect_timeout)
            .build()
            .expect("failed to build HTTP client");
        Self {
            client,
            base_url,
            bearer_token: None,
        }
    }

    /// Set a bearer token for the Authorization header (e.g. a service token).
    pub fn with_bearer_token(mut self, token: String) -> Self {
        self.bearer_token = Some(token);
        self
    }

    /// Create service-level claims for module-to-module calls that don't
    /// originate from an HTTP request (e.g. event consumers, background tasks).
    ///
    /// Uses the provided tenant_id and the service's own identity. For HTTP
    /// request handlers, pass the inbound `VerifiedClaims` directly instead.
    pub fn service_claims(tenant_id: uuid::Uuid) -> VerifiedClaims {
        VerifiedClaims {
            user_id: uuid::Uuid::nil(),
            tenant_id,
            app_id: None,
            roles: vec![],
            perms: vec!["service.internal".to_string()],
            actor_type: security::claims::ActorType::Service,
            issued_at: Utc::now(),
            expires_at: Utc::now() + chrono::TimeDelta::hours(1),
            token_id: uuid::Uuid::new_v4(),
            version: "1.0".to_string(),
        }
    }

    /// GET — retries on 429/503 (safe, idempotent).
    pub async fn get(&self, path: &str, claims: &VerifiedClaims) -> Result<Response, reqwest::Error> {
        self.send_with_retry(self.client.get(self.url(path)), claims).await
    }

    /// POST — no retry (mutations are not safe to retry).
    pub async fn post<T: Serialize>(&self, path: &str, body: &T, claims: &VerifiedClaims) -> Result<Response, reqwest::Error> {
        self.send_once(self.client.post(self.url(path)).json(body), claims).await
    }

    /// PUT — no retry (mutations are not safe to retry).
    pub async fn put<T: Serialize>(&self, path: &str, body: &T, claims: &VerifiedClaims) -> Result<Response, reqwest::Error> {
        self.send_once(self.client.put(self.url(path)).json(body), claims).await
    }

    /// PATCH — no retry (mutations are not safe to retry).
    pub async fn patch<T: Serialize>(&self, path: &str, body: &T, claims: &VerifiedClaims) -> Result<Response, reqwest::Error> {
        self.send_once(self.client.patch(self.url(path)).json(body), claims).await
    }

    /// DELETE — no retry (mutations are not safe to retry).
    pub async fn delete(&self, path: &str, claims: &VerifiedClaims) -> Result<Response, reqwest::Error> {
        self.send_once(self.client.delete(self.url(path)), claims).await
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    fn inject_headers(&self, mut req: reqwest::RequestBuilder, claims: &VerifiedClaims, correlation_id: &Uuid) -> reqwest::RequestBuilder {
        req = req
            .header("x-tenant-id", claims.tenant_id.to_string())
            .header("x-correlation-id", correlation_id.to_string())
            .header("x-actor-id", claims.user_id.to_string());

        if let Some(app_id) = &claims.app_id {
            req = req.header("x-app-id", app_id.to_string());
        }

        if let Some(token) = &self.bearer_token {
            req = req.header("authorization", format!("Bearer {token}"));
        }

        req
    }

    /// Send once without retry — for mutations (POST/PUT/PATCH/DELETE).
    async fn send_once(
        &self,
        builder: reqwest::RequestBuilder,
        claims: &VerifiedClaims,
    ) -> Result<Response, reqwest::Error> {
        let correlation_id = Uuid::new_v4();
        let req = self.inject_headers(builder, claims, &correlation_id);
        req.send().await
    }

    /// Send with retry on 429/503 and transient transport errors — for reads (GET) only.
    ///
    /// Retries on: HTTP 429, HTTP 503, connection refused, timeouts.
    /// Does NOT retry on: DNS failures (permanent), TLS errors (permanent).
    async fn send_with_retry(
        &self,
        builder: reqwest::RequestBuilder,
        claims: &VerifiedClaims,
    ) -> Result<Response, reqwest::Error> {
        let correlation_id = Uuid::new_v4();
        let mut backoff = Duration::from_millis(INITIAL_BACKOFF_MS);
        let mut last_err: Option<reqwest::Error> = None;

        for attempt in 0..=MAX_RETRIES {
            let req = builder.try_clone().expect("request must be cloneable");
            let req = self.inject_headers(req, claims, &correlation_id);

            match req.send().await {
                Ok(resp) => {
                    let status = resp.status();
                    if attempt < MAX_RETRIES && (status == StatusCode::TOO_MANY_REQUESTS || status == StatusCode::SERVICE_UNAVAILABLE) {
                        tracing::warn!(attempt, status = %status, "retrying after HTTP status");
                        tokio::time::sleep(backoff).await;
                        backoff *= 2;
                        continue;
                    }
                    return Ok(resp);
                }
                Err(e) => {
                    if attempt < MAX_RETRIES && is_transient_transport_error(&e) {
                        tracing::warn!(attempt, error = %e, "retrying after transient transport error");
                        tokio::time::sleep(backoff).await;
                        backoff *= 2;
                        last_err = Some(e);
                        continue;
                    }
                    return Err(e);
                }
            }
        }

        Err(last_err.expect("loop must have set last_err before exhausting retries"))
    }
}

/// Returns true for transient transport errors worth retrying (connection refused, timeout).
/// Returns false for permanent errors (DNS resolution, TLS certificate issues).
fn is_transient_transport_error(err: &reqwest::Error) -> bool {
    if err.is_timeout() {
        return true;
    }
    if err.is_connect() {
        // DNS failures are permanent — do not retry.
        // reqwest wraps hyper errors; DNS failures contain "dns error" in the chain.
        let msg = err.to_string();
        if msg.contains("dns error") || msg.contains("failed to lookup address") {
            return false;
        }
        // Connection refused, reset, etc. are transient.
        return true;
    }
    false
}
