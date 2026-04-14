//! Typed HTTP client for calling platform services with auto-injected headers, retry,
//! circuit breaker, and bulkhead isolation.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use reqwest::{Client, Response, StatusCode};
use serde::Serialize;
use tokio::sync::Semaphore;
use uuid::Uuid;

use chrono::Utc;
use security::claims::VerifiedClaims;

// ── Timeout config ────────────────────────────────────────────────────────────

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

// ── Circuit breaker config ────────────────────────────────────────────────────

/// Resilience configuration for a `PlatformClient`.
///
/// Controls the circuit breaker (opens on sustained failures) and the bulkhead
/// (caps concurrent outbound requests to prevent pool exhaustion).
#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    /// Number of consecutive failures that trip the circuit (default: 3).
    pub consecutive_failures_threshold: u32,
    /// Error rate (0.0–1.0) within the window that also trips the circuit (default: 0.50).
    pub error_rate_threshold: f64,
    /// Sliding window for error-rate tracking (default: 10s).
    pub error_rate_window: Duration,
    /// Minimum requests in the window before the error-rate rule applies (default: 5).
    pub min_requests_in_window: usize,
    /// How long the circuit stays open before probing with a half-open request (default: 30s).
    pub open_duration: Duration,
    /// Maximum concurrent outbound requests — bulkhead capacity (default: 5).
    pub bulkhead_capacity: usize,
    /// How long to wait for a bulkhead slot before returning 503 (default: 2s).
    pub bulkhead_wait: Duration,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            consecutive_failures_threshold: 3,
            error_rate_threshold: 0.50,
            error_rate_window: Duration::from_secs(10),
            min_requests_in_window: 5,
            open_duration: Duration::from_secs(30),
            bulkhead_capacity: 5,
            bulkhead_wait: Duration::from_secs(2),
        }
    }
}

// ── Circuit breaker internals ─────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
enum CBState {
    Closed,
    Open,
    HalfOpen,
}

struct CBInner {
    state: CBState,
    consecutive_failures: u32,
    /// Monotonic clock timestamp when the circuit opened.
    opened_at: Option<Instant>,
    /// Wall-clock timestamp when the circuit opened (for health display).
    opened_at_wall: Option<chrono::DateTime<chrono::Utc>>,
    /// True while a half-open probe request is in flight.
    probe_in_flight: bool,
    /// When the current probe started (guards against task-cancellation leaks).
    probe_started: Option<Instant>,
    /// Sliding window of (monotonic timestamp, is_failure).
    window: VecDeque<(Instant, bool)>,
}

impl CBInner {
    fn new() -> Self {
        Self {
            state: CBState::Closed,
            consecutive_failures: 0,
            opened_at: None,
            opened_at_wall: None,
            probe_in_flight: false,
            probe_started: None,
            window: VecDeque::new(),
        }
    }

    /// Returns `true` if the request should be allowed through, `false` to reject it.
    ///
    /// Side-effects:
    /// - Open → HalfOpen when `open_duration` has elapsed.
    /// - Sets `probe_in_flight = true` for the single half-open probe.
    fn gate(&mut self, config: &CircuitBreakerConfig) -> bool {
        match self.state {
            CBState::Closed => true,
            CBState::Open => {
                if self
                    .opened_at
                    .map_or(false, |t| t.elapsed() >= config.open_duration)
                {
                    self.state = CBState::HalfOpen;
                    self.probe_in_flight = true;
                    self.probe_started = Some(Instant::now());
                    tracing::info!("circuit breaker transitioning to half-open");
                    true
                } else {
                    false
                }
            }
            CBState::HalfOpen => {
                if self.probe_in_flight {
                    // Safety valve: if a probe has been in flight longer than open_duration
                    // (task cancellation), reset and allow a new probe.
                    let stale = self
                        .probe_started
                        .map_or(false, |t| t.elapsed() > config.open_duration);
                    if stale {
                        tracing::warn!("half-open probe stale — resetting");
                        self.probe_started = Some(Instant::now());
                        true
                    } else {
                        false
                    }
                } else {
                    self.probe_in_flight = true;
                    self.probe_started = Some(Instant::now());
                    true
                }
            }
        }
    }

    /// Record the outcome of a completed request.
    fn record(&mut self, failed: bool, config: &CircuitBreakerConfig) {
        let now = Instant::now();

        match self.state {
            CBState::HalfOpen => {
                self.probe_in_flight = false;
                self.probe_started = None;
                if failed {
                    self.open_circuit(now);
                    tracing::warn!("circuit breaker half-open probe failed — reopening");
                } else {
                    self.state = CBState::Closed;
                    self.consecutive_failures = 0;
                    self.opened_at = None;
                    self.opened_at_wall = None;
                    tracing::info!("circuit breaker closed after successful half-open probe");
                }
            }
            CBState::Closed => {
                // Prune stale window entries
                let cutoff = now.checked_sub(config.error_rate_window).unwrap_or(now);
                while self.window.front().map_or(false, |(t, _)| *t < cutoff) {
                    self.window.pop_front();
                }
                self.window.push_back((now, failed));

                if failed {
                    self.consecutive_failures += 1;
                    if self.should_open(config) {
                        self.open_circuit(now);
                    }
                } else {
                    self.consecutive_failures = 0;
                }
            }
            CBState::Open => {
                // gate() rejects Open-state requests before record() is reached.
            }
        }
    }

    fn should_open(&self, config: &CircuitBreakerConfig) -> bool {
        if self.consecutive_failures >= config.consecutive_failures_threshold {
            return true;
        }
        let total = self.window.len();
        if total >= config.min_requests_in_window {
            let failures = self.window.iter().filter(|(_, f)| *f).count();
            let rate = failures as f64 / total as f64;
            if rate >= config.error_rate_threshold {
                return true;
            }
        }
        false
    }

    fn open_circuit(&mut self, now: Instant) {
        self.state = CBState::Open;
        self.opened_at = Some(now);
        self.opened_at_wall = Some(Utc::now());
        tracing::warn!(
            consecutive_failures = self.consecutive_failures,
            "circuit breaker opened"
        );
    }
}

struct CircuitBreaker {
    inner: Mutex<CBInner>,
    config: CircuitBreakerConfig,
}

impl CircuitBreaker {
    fn new(config: CircuitBreakerConfig) -> Self {
        Self {
            inner: Mutex::new(CBInner::new()),
            config,
        }
    }
}

// ── Synthetic response helpers ────────────────────────────────────────────────

fn synthetic_503_circuit_open(retry_after_secs: u64) -> Response {
    let body = format!(
        r#"{{"error":"circuit_open","message":"Service circuit breaker is open. Retry after {}s."}}"#,
        retry_after_secs
    );
    let resp = http::Response::builder()
        .status(503)
        .header("content-type", "application/json")
        .header("retry-after", retry_after_secs.to_string())
        .body(body)
        .expect("synthetic circuit-open 503");
    Response::from(resp)
}

fn synthetic_503_bulkhead() -> Response {
    let resp = http::Response::builder()
        .status(503)
        .header("content-type", "application/json")
        .body(
            r#"{"error":"service_unavailable","message":"Outbound connection pool exhausted"}"#
                .to_string(),
        )
        .expect("synthetic bulkhead 503");
    Response::from(resp)
}

// ── Failure classification ────────────────────────────────────────────────────

/// Returns `true` for outcomes that should count against the circuit breaker.
///
/// Failures: transport errors (timeout, connection refused) and HTTP 5xx responses.
/// Not failures: HTTP 4xx (client mistakes, including 429 rate-limiting).
fn is_circuit_failure(result: &Result<Response, reqwest::Error>) -> bool {
    match result {
        Err(e) => e.is_timeout() || (e.is_connect() && !is_dns_error(e)),
        Ok(resp) => resp.status().is_server_error(),
    }
}

fn is_dns_error(err: &reqwest::Error) -> bool {
    let msg = err.to_string();
    msg.contains("dns error") || msg.contains("failed to lookup address")
}

// ── PlatformClient ────────────────────────────────────────────────────────────

/// HTTP client that injects platform headers, retries on 429/503, and adds
/// per-service circuit breaking and bulkhead isolation.
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
    /// Shared circuit breaker state — all clones of this client share one breaker.
    cb: Arc<CircuitBreaker>,
    /// Bulkhead semaphore — caps concurrent outbound requests per target service.
    bulkhead: Arc<Semaphore>,
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
        let cb_config = CircuitBreakerConfig::default();
        let bulkhead = Arc::new(Semaphore::new(cb_config.bulkhead_capacity));
        let cb = Arc::new(CircuitBreaker::new(cb_config));
        Self {
            client,
            base_url,
            bearer_token: None,
            cb,
            bulkhead,
        }
    }

    /// Replace the circuit breaker / bulkhead configuration.
    ///
    /// Creates a fresh circuit breaker and bulkhead with the supplied settings.
    /// Call this immediately after construction before any requests are made.
    pub fn with_cb_config(self, config: CircuitBreakerConfig) -> Self {
        let bulkhead = Arc::new(Semaphore::new(config.bulkhead_capacity));
        let cb = Arc::new(CircuitBreaker::new(config));
        Self {
            bulkhead,
            cb,
            ..self
        }
    }

    /// Set a bearer token for the Authorization header (e.g. a service token).
    pub fn with_bearer_token(mut self, token: String) -> Self {
        self.bearer_token = Some(token);
        self
    }

    /// Return the current circuit breaker state for inclusion in `/api/ready`.
    ///
    /// The `service_name` label identifies this downstream in the health response
    /// (e.g. `"bom"`, `"production"`).
    pub fn circuit_status(&self, service_name: &str) -> health::CircuitBreakerInfo {
        let inner = self.cb.inner.lock().expect("circuit breaker lock poisoned");
        health::CircuitBreakerInfo {
            service: service_name.to_string(),
            state: match inner.state {
                CBState::Closed => "closed".to_string(),
                CBState::Open => "open".to_string(),
                CBState::HalfOpen => "half_open".to_string(),
            },
            consecutive_failures: inner.consecutive_failures,
            open_since: inner.opened_at_wall.map(|t| t.to_rfc3339()),
        }
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
    pub async fn get(
        &self,
        path: &str,
        claims: &VerifiedClaims,
    ) -> Result<Response, reqwest::Error> {
        if let Some(resp) = self.circuit_gate() {
            return Ok(resp);
        }
        let _permit = match self.bulkhead_acquire().await {
            Ok(p) => p,
            Err(resp) => return Ok(resp),
        };
        let result = self
            .send_with_retry(self.client.get(self.url(path)), claims)
            .await;
        self.record_circuit_outcome(&result);
        result
    }

    /// POST — no retry (mutations are not safe to retry).
    pub async fn post<T: Serialize>(
        &self,
        path: &str,
        body: &T,
        claims: &VerifiedClaims,
    ) -> Result<Response, reqwest::Error> {
        if let Some(resp) = self.circuit_gate() {
            return Ok(resp);
        }
        let _permit = match self.bulkhead_acquire().await {
            Ok(p) => p,
            Err(resp) => return Ok(resp),
        };
        let result = self
            .send_once(self.client.post(self.url(path)).json(body), claims)
            .await;
        self.record_circuit_outcome(&result);
        result
    }

    /// PUT — no retry (mutations are not safe to retry).
    pub async fn put<T: Serialize>(
        &self,
        path: &str,
        body: &T,
        claims: &VerifiedClaims,
    ) -> Result<Response, reqwest::Error> {
        if let Some(resp) = self.circuit_gate() {
            return Ok(resp);
        }
        let _permit = match self.bulkhead_acquire().await {
            Ok(p) => p,
            Err(resp) => return Ok(resp),
        };
        let result = self
            .send_once(self.client.put(self.url(path)).json(body), claims)
            .await;
        self.record_circuit_outcome(&result);
        result
    }

    /// PATCH — no retry (mutations are not safe to retry).
    pub async fn patch<T: Serialize>(
        &self,
        path: &str,
        body: &T,
        claims: &VerifiedClaims,
    ) -> Result<Response, reqwest::Error> {
        if let Some(resp) = self.circuit_gate() {
            return Ok(resp);
        }
        let _permit = match self.bulkhead_acquire().await {
            Ok(p) => p,
            Err(resp) => return Ok(resp),
        };
        let result = self
            .send_once(self.client.patch(self.url(path)).json(body), claims)
            .await;
        self.record_circuit_outcome(&result);
        result
    }

    /// DELETE — no retry (mutations are not safe to retry).
    pub async fn delete(
        &self,
        path: &str,
        claims: &VerifiedClaims,
    ) -> Result<Response, reqwest::Error> {
        if let Some(resp) = self.circuit_gate() {
            return Ok(resp);
        }
        let _permit = match self.bulkhead_acquire().await {
            Ok(p) => p,
            Err(resp) => return Ok(resp),
        };
        let result = self
            .send_once(self.client.delete(self.url(path)), claims)
            .await;
        self.record_circuit_outcome(&result);
        result
    }

    // -- Anonymous variants (no VerifiedClaims required) --

    /// GET without auth headers — for public/pre-auth endpoints.
    pub async fn get_anon(&self, path: &str) -> Result<Response, reqwest::Error> {
        if let Some(resp) = self.circuit_gate() {
            return Ok(resp);
        }
        let _permit = match self.bulkhead_acquire().await {
            Ok(p) => p,
            Err(resp) => return Ok(resp),
        };
        let result = self
            .send_with_retry_anon(self.client.get(self.url(path)))
            .await;
        self.record_circuit_outcome(&result);
        result
    }

    /// POST without auth headers — for public/pre-auth endpoints.
    pub async fn post_anon<T: Serialize>(
        &self,
        path: &str,
        body: &T,
    ) -> Result<Response, reqwest::Error> {
        if let Some(resp) = self.circuit_gate() {
            return Ok(resp);
        }
        let _permit = match self.bulkhead_acquire().await {
            Ok(p) => p,
            Err(resp) => return Ok(resp),
        };
        let result = self
            .send_once_anon(self.client.post(self.url(path)).json(body))
            .await;
        self.record_circuit_outcome(&result);
        result
    }

    /// PUT without auth headers — for public/pre-auth endpoints.
    pub async fn put_anon<T: Serialize>(
        &self,
        path: &str,
        body: &T,
    ) -> Result<Response, reqwest::Error> {
        if let Some(resp) = self.circuit_gate() {
            return Ok(resp);
        }
        let _permit = match self.bulkhead_acquire().await {
            Ok(p) => p,
            Err(resp) => return Ok(resp),
        };
        let result = self
            .send_once_anon(self.client.put(self.url(path)).json(body))
            .await;
        self.record_circuit_outcome(&result);
        result
    }

    /// PATCH without auth headers — for public/pre-auth endpoints.
    pub async fn patch_anon<T: Serialize>(
        &self,
        path: &str,
        body: &T,
    ) -> Result<Response, reqwest::Error> {
        if let Some(resp) = self.circuit_gate() {
            return Ok(resp);
        }
        let _permit = match self.bulkhead_acquire().await {
            Ok(p) => p,
            Err(resp) => return Ok(resp),
        };
        let result = self
            .send_once_anon(self.client.patch(self.url(path)).json(body))
            .await;
        self.record_circuit_outcome(&result);
        result
    }

    /// DELETE without auth headers — for public/pre-auth endpoints.
    pub async fn delete_anon(&self, path: &str) -> Result<Response, reqwest::Error> {
        if let Some(resp) = self.circuit_gate() {
            return Ok(resp);
        }
        let _permit = match self.bulkhead_acquire().await {
            Ok(p) => p,
            Err(resp) => return Ok(resp),
        };
        let result = self
            .send_once_anon(self.client.delete(self.url(path)))
            .await;
        self.record_circuit_outcome(&result);
        result
    }

    // -- Private helpers --

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    /// Check the circuit breaker gate. Returns `Some(503)` if rejected, `None` if allowed.
    fn circuit_gate(&self) -> Option<Response> {
        let mut inner = self.cb.inner.lock().expect("circuit breaker lock poisoned");
        if inner.gate(&self.cb.config) {
            None
        } else {
            Some(synthetic_503_circuit_open(
                self.cb.config.open_duration.as_secs(),
            ))
        }
    }

    /// Acquire a bulkhead slot (up to `bulkhead_wait`).
    /// Returns `Ok(permit)` on success or `Err(503)` when the pool is exhausted.
    async fn bulkhead_acquire(&self) -> Result<tokio::sync::OwnedSemaphorePermit, Response> {
        match tokio::time::timeout(
            self.cb.config.bulkhead_wait,
            Arc::clone(&self.bulkhead).acquire_owned(),
        )
        .await
        {
            Ok(Ok(permit)) => Ok(permit),
            _ => Err(synthetic_503_bulkhead()),
        }
    }

    /// Record the outcome of a completed real request against the circuit breaker.
    fn record_circuit_outcome(&self, result: &Result<Response, reqwest::Error>) {
        let failed = is_circuit_failure(result);
        let mut inner = self.cb.inner.lock().expect("circuit breaker lock poisoned");
        inner.record(failed, &self.cb.config);
    }

    fn inject_headers(
        &self,
        mut req: reqwest::RequestBuilder,
        claims: &VerifiedClaims,
        correlation_id: &Uuid,
    ) -> reqwest::RequestBuilder {
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
        let token = security::service_auth::mint_service_jwt_with_context(
            claims.tenant_id,
            claims.user_id,
        )
        .map_err(
            |e| tracing::warn!(error = %e, "failed to mint service JWT for cross-service call"),
        )
        .ok()
        .or_else(|| self.bearer_token.clone());

        if let Some(token) = token {
            req = req.header("authorization", format!("Bearer {token}"));
        }

        req
    }

    fn inject_anon_headers(
        &self,
        mut req: reqwest::RequestBuilder,
        correlation_id: &Uuid,
    ) -> reqwest::RequestBuilder {
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
    fn inject_trace_headers(
        &self,
        mut req: reqwest::RequestBuilder,
        correlation_id: &Uuid,
    ) -> reqwest::RequestBuilder {
        let trace_id = crate::startup::CURRENT_TRACE_ID
            .try_with(|id| id.clone())
            .ok();
        if let Some(ref tid) = trace_id {
            let trace_hex = tid.replace('-', "");
            if trace_hex.len() == 32 {
                // Derive a stable span_id from the first 8 bytes of the correlation UUID.
                let bytes = correlation_id.as_bytes();
                let span_id_val = u64::from_be_bytes([
                    bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
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
                    if attempt < MAX_RETRIES
                        && (status == StatusCode::TOO_MANY_REQUESTS
                            || status == StatusCode::SERVICE_UNAVAILABLE)
                    {
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
                    if attempt < MAX_RETRIES
                        && (status == StatusCode::TOO_MANY_REQUESTS
                            || status == StatusCode::SERVICE_UNAVAILABLE)
                    {
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
