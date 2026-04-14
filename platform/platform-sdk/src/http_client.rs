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

    /// Like [`service_claims`](Self::service_claims), but parses a string
    /// tenant ID into a UUID. Eliminates the `Uuid::parse_str` boilerplate
    /// verticals otherwise need when their tenant IDs arrive as strings.
    pub fn service_claims_from_str(tenant_id: &str) -> Result<VerifiedClaims, uuid::Error> {
        let tenant_id = uuid::Uuid::parse_str(tenant_id)?;
        Ok(Self::service_claims(tenant_id))
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

    // -- Anonymous variants (no VerifiedClaims required) --

    /// GET without auth headers — for public/pre-auth endpoints.
    pub async fn get_anon(&self, path: &str) -> Result<Response, reqwest::Error> {
        self.send_with_retry_anon(self.client.get(self.url(path))).await
    }

    /// POST without auth headers — for public/pre-auth endpoints.
    pub async fn post_anon<T: Serialize>(&self, path: &str, body: &T) -> Result<Response, reqwest::Error> {
        self.send_once_anon(self.client.post(self.url(path)).json(body)).await
    }

    /// PUT without auth headers — for public/pre-auth endpoints.
    pub async fn put_anon<T: Serialize>(&self, path: &str, body: &T) -> Result<Response, reqwest::Error> {
        self.send_once_anon(self.client.put(self.url(path)).json(body)).await
    }

    /// PATCH without auth headers — for public/pre-auth endpoints.
    pub async fn patch_anon<T: Serialize>(&self, path: &str, body: &T) -> Result<Response, reqwest::Error> {
        self.send_once_anon(self.client.patch(self.url(path)).json(body)).await
    }

    /// DELETE without auth headers — for public/pre-auth endpoints.
    pub async fn delete_anon(&self, path: &str) -> Result<Response, reqwest::Error> {
        self.send_once_anon(self.client.delete(self.url(path))).await
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    fn inject_headers(&self, mut req: reqwest::RequestBuilder, claims: &VerifiedClaims, correlation_id: &Uuid) -> reqwest::RequestBuilder {
        // Propagate the current distributed trace context so downstream services
        // produce child spans under the same trace_id.  The trace_id is stored in a
        // task-local by platform_trace_middleware and is available here without
        // requiring callers to thread it through every method signature.
        req = self.inject_trace_headers(req, correlation_id);

        req = req
            .header("x-tenant-id", claims.tenant_id.to_string())
            .header("x-correlation-id", correlation_id.to_string())
            .header("x-actor-id", claims.user_id.to_string());

        if let Some(app_id) = &claims.app_id {
            req = req.header("x-app-id", app_id.to_string());
        }

        // Prefer a per-request service JWT so the receiving service sees the
        // caller's real tenant_id and actor_id rather than the nil UUIDs that
        // the startup bearer token carries.  Fall back to the static bearer
        // token only when JWT minting is unavailable (e.g. no private key).
        let token = security::service_auth::mint_service_jwt_with_context(claims.tenant_id, claims.user_id)
            .map_err(|e| tracing::warn!(error = %e, "failed to mint service JWT for cross-service call"))
            .ok()
            .or_else(|| self.bearer_token.clone());

        if let Some(token) = token {
            req = req.header("authorization", format!("Bearer {token}"));
        }

        req
    }

    fn inject_anon_headers(&self, mut req: reqwest::RequestBuilder, correlation_id: &Uuid) -> reqwest::RequestBuilder {
        req = self.inject_trace_headers(req, correlation_id);
        req = req.header("x-correlation-id", correlation_id.to_string());
        if let Some(token) = &self.bearer_token {
            req = req.header("authorization", format!("Bearer {token}"));
        }
        req
    }

    /// Inject W3C `traceparent` and `X-Trace-Id` headers if a trace context is active.
    ///
    /// The trace ID comes from the [`crate::startup::CURRENT_TRACE_ID`] task-local set
    /// by `platform_trace_middleware` on the inbound request.  If no trace context is
    /// active (e.g. in background tasks), this is a no-op.
    ///
    /// `traceparent` format: `00-{trace_id_32hex}-{span_id_16hex}-01`
    ///   - trace_id: the 128-bit request trace ID as 32 lowercase hex chars
    ///   - span_id:  a fresh 64-bit ID for this outbound call (first 8 bytes of correlation_id)
    ///   - flags:    `01` = sampled
    fn inject_trace_headers(&self, mut req: reqwest::RequestBuilder, correlation_id: &Uuid) -> reqwest::RequestBuilder {
        let trace_id = crate::startup::CURRENT_TRACE_ID.try_with(|id| id.clone()).ok();
        if let Some(ref tid) = trace_id {
            let trace_hex = tid.replace('-', "");
            if trace_hex.len() == 32 {
                // Derive a stable span_id from the first 8 bytes of the correlation UUID.
                let bytes = correlation_id.as_bytes();
                let span_id_val = u64::from_be_bytes([
                    bytes[0], bytes[1], bytes[2], bytes[3],
                    bytes[4], bytes[5], bytes[6], bytes[7],
                ]);
                let span_hex = format!("{:016x}", span_id_val);
                let traceparent = format!("00-{}-{}-01", trace_hex, span_hex);
                req = req.header("traceparent", traceparent);
            }
            req = req.header("x-trace-id", tid.as_str());
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

    /// Send once without retry or auth headers — for anonymous mutations.
    async fn send_once_anon(
        &self,
        builder: reqwest::RequestBuilder,
    ) -> Result<Response, reqwest::Error> {
        let correlation_id = Uuid::new_v4();
        let req = self.inject_anon_headers(builder, &correlation_id);
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

    /// Retry loop for anonymous GET requests (no auth headers).
    async fn send_with_retry_anon(
        &self,
        builder: reqwest::RequestBuilder,
    ) -> Result<Response, reqwest::Error> {
        let correlation_id = Uuid::new_v4();
        let mut backoff = Duration::from_millis(INITIAL_BACKOFF_MS);
        let mut last_err: Option<reqwest::Error> = None;

        for attempt in 0..=MAX_RETRIES {
            let req = builder.try_clone().expect("request must be cloneable");
            let req = self.inject_anon_headers(req, &correlation_id);

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
