//! Circuit breaker and bulkhead E2E tests for PlatformClient (bd-wpxqn)
//!
//! Tests the full circuit breaker lifecycle against a real in-process HTTP server:
//!
//! 1. Circuit opens after 3 consecutive 503 responses
//! 2. Open circuit returns instant synthetic 503 (no server round-trip)
//! 3. Unrelated client (different PlatformClient) is unaffected
//! 4. Circuit recovers to half-open after `open_duration`, probe succeeds → closed
//! 5. Bulkhead caps concurrent requests; 6th waits then gets 503
//! 6. `circuit_status()` reports state visible in `/api/ready`
//!
//! ## Running
//! ```bash
//! ./scripts/cargo-slot.sh test -p e2e-tests circuit_breaker_e2e -- --nocapture
//! ```

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{extract::State, routing::get, Json, Router};
use platform_sdk::{CircuitBreakerConfig, PlatformClient, TimeoutConfig};
use reqwest::StatusCode;
use serde_json::Value;
use tokio::net::TcpListener;

// ── Test server ───────────────────────────────────────────────────────────────

/// Shared counter so we can verify circuit-open paths don't hit the server.
#[derive(Clone, Default)]
struct HitCounter(Arc<AtomicU32>);

impl HitCounter {
    fn increment(&self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
    fn get(&self) -> u32 {
        self.0.load(Ordering::Relaxed)
    }
}

/// Bind a fresh ephemeral port and spawn an axum server.
///
/// Routes:
/// - `GET /fail`  → 503 Service Unavailable
/// - `GET /ok`    → 200 OK
/// - `GET /slow`  → 200 OK after 500ms (used for bulkhead test)
///
/// Returns (base_url, hit_counter_for_fail, hit_counter_for_ok).
async fn start_test_server() -> (String, HitCounter, HitCounter) {
    let fail_hits = HitCounter::default();
    let ok_hits = HitCounter::default();

    let router = Router::new()
        .route("/fail", get(fail_handler))
        .route("/ok", get(ok_handler))
        .route("/slow", get(slow_handler))
        .with_state((fail_hits.clone(), ok_hits.clone()));

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let port = listener.local_addr().expect("local_addr").port();
    let base_url = format!("http://127.0.0.1:{}", port);

    tokio::spawn(async move {
        axum::serve(listener, router)
            .await
            .expect("test server error");
    });

    // Give the server a moment to accept connections
    tokio::time::sleep(Duration::from_millis(30)).await;

    (base_url, fail_hits, ok_hits)
}

async fn fail_handler(
    State((fail_hits, _ok_hits)): State<(HitCounter, HitCounter)>,
) -> axum::http::StatusCode {
    fail_hits.increment();
    axum::http::StatusCode::SERVICE_UNAVAILABLE
}

async fn ok_handler(
    State((_fail_hits, ok_hits)): State<(HitCounter, HitCounter)>,
) -> (axum::http::StatusCode, Json<Value>) {
    ok_hits.increment();
    (
        axum::http::StatusCode::OK,
        Json(serde_json::json!({"status": "ok"})),
    )
}

async fn slow_handler() -> (axum::http::StatusCode, Json<Value>) {
    tokio::time::sleep(Duration::from_millis(500)).await;
    (
        axum::http::StatusCode::OK,
        Json(serde_json::json!({"status": "slow-ok"})),
    )
}

// ── Test config builder ───────────────────────────────────────────────────────

/// Build a PlatformClient with short timings suitable for testing.
///
/// open_duration: 2s so recovery tests don't take forever.
fn test_client(base_url: String) -> PlatformClient {
    PlatformClient::with_timeout(
        base_url,
        TimeoutConfig {
            request_timeout: Duration::from_secs(5),
            connect_timeout: Duration::from_secs(2),
        },
    )
    .with_cb_config(CircuitBreakerConfig {
        consecutive_failures_threshold: 3,
        error_rate_threshold: 0.50,
        error_rate_window: Duration::from_secs(10),
        min_requests_in_window: 5,
        open_duration: Duration::from_secs(2), // short for testing
        bulkhead_capacity: 5,
        bulkhead_wait: Duration::from_millis(200),
    })
}

// ── Minimal fake claims for PlatformClient auth methods ──────────────────────

fn fake_claims() -> platform_sdk::VerifiedClaims {
    PlatformClient::service_claims(uuid::Uuid::nil())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Circuit opens after 3 consecutive 503 responses from the downstream service.
#[tokio::test]
async fn test_circuit_opens_after_three_consecutive_failures() {
    let (base_url, fail_hits, _ok_hits) = start_test_server().await;
    let client = test_client(base_url);
    let claims = fake_claims();

    // Exhaust retries with MAX_RETRIES=3, so each call makes up to 4 attempts.
    // The send_with_retry retries on 503, so each top-level get() call hammers
    // the server multiple times.  For the circuit breaker we care about the
    // final outcome per top-level call.  After 3 top-level failures → open.

    for call in 1..=3u32 {
        let resp = client
            .get("/fail", &claims)
            .await
            .expect("get must succeed");
        assert_eq!(
            resp.status(),
            StatusCode::SERVICE_UNAVAILABLE,
            "call {} should get 503 from server",
            call
        );
    }

    // Verify server actually received calls (not short-circuited yet)
    assert!(
        fail_hits.get() > 0,
        "server must have been hit before circuit opened"
    );

    // 4th call: circuit should now be open → instant synthetic 503
    let server_hits_before = fail_hits.get();
    let start = Instant::now();
    let resp = client
        .get("/fail", &claims)
        .await
        .expect("get must succeed");
    let elapsed = start.elapsed();

    assert_eq!(
        resp.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "open circuit must return 503"
    );
    // Collect headers before consuming `resp` with `.json()`
    let has_retry_after = resp.headers().contains_key("retry-after");
    let body: Value = resp.json().await.expect("parse body");
    assert_eq!(
        body["error"], "circuit_open",
        "error field must be circuit_open"
    );

    // Open circuit must NOT hit the server
    assert_eq!(
        fail_hits.get(),
        server_hits_before,
        "open circuit must not hit the server"
    );

    // Must be fast (synthetic response, no network round-trip)
    assert!(
        elapsed < Duration::from_millis(100),
        "open circuit 503 must be fast, took {:?}",
        elapsed
    );

    // retry-after header must be present
    assert!(
        has_retry_after,
        "retry-after header must be present on circuit-open 503"
    );

    println!(
        "PASS: circuit opened after 3 failures, fast 503 returned (elapsed={:?})",
        elapsed
    );
}

/// A second PlatformClient (different base service) is unaffected when another
/// client's circuit is open.  This verifies per-client (per-service) isolation.
#[tokio::test]
async fn test_unrelated_client_unaffected() {
    let (base_url, _fail_hits, _ok_hits) = start_test_server().await;

    // Client A points at /fail, trips its circuit
    let client_a = test_client(base_url.clone());
    let claims = fake_claims();

    for _ in 0..3 {
        let _ = client_a.get("/fail", &claims).await;
    }

    // Verify client A is tripped
    let resp_a = client_a.get("/fail", &claims).await.unwrap();
    assert_eq!(resp_a.status(), 503);
    let body: Value = resp_a.json().await.unwrap();
    assert_eq!(body["error"], "circuit_open");

    // Client B is a SEPARATE PlatformClient pointing to the same server's /ok path.
    // Its own circuit is fresh-closed.
    let client_b = test_client(base_url);
    let resp_b = client_b
        .get("/ok", &claims)
        .await
        .expect("client B must succeed");
    assert_eq!(
        resp_b.status(),
        StatusCode::OK,
        "client B (separate circuit) must be unaffected"
    );

    println!("PASS: client A circuit open, client B unaffected");
}

/// Circuit recovers: after open_duration (2s in test), a probe succeeds and
/// the circuit transitions Closed.
#[tokio::test]
async fn test_circuit_recovers_after_open_duration() {
    let (base_url, fail_hits, ok_hits) = start_test_server().await;
    let client = test_client(base_url);
    let claims = fake_claims();

    // Trip the circuit
    for _ in 0..3 {
        let _ = client.get("/fail", &claims).await;
    }

    // Verify open
    let resp = client.get("/fail", &claims).await.unwrap();
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["error"], "circuit_open", "circuit must be open");

    // Check status via circuit_status()
    let status = client.circuit_status("test-service");
    assert_eq!(status.state, "open");
    assert!(status.open_since.is_some());
    println!("  circuit state: {:?}", status);

    // Wait for open_duration (2s) to expire
    tokio::time::sleep(Duration::from_millis(2100)).await;

    // Next request should be a half-open probe to /ok (which returns 200)
    let ok_hits_before = ok_hits.get();
    let resp = client
        .get("/ok", &claims)
        .await
        .expect("probe must not error");
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "half-open probe to /ok must succeed"
    );
    assert!(
        ok_hits.get() > ok_hits_before,
        "probe must have reached the server"
    );

    // Circuit should now be closed
    let status = client.circuit_status("test-service");
    assert_eq!(
        status.state, "closed",
        "circuit must be closed after successful probe"
    );
    assert_eq!(status.consecutive_failures, 0);

    // Further requests to /ok must continue to work normally
    let resp = client.get("/ok", &claims).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    println!(
        "PASS: circuit recovered after open_duration; fail_hits={}, ok_hits={}",
        fail_hits.get(),
        ok_hits.get()
    );
}

/// Half-open probe failure re-opens the circuit.
#[tokio::test]
async fn test_half_open_probe_failure_reopens_circuit() {
    let (base_url, _fail_hits, _ok_hits) = start_test_server().await;
    let client = test_client(base_url);
    let claims = fake_claims();

    // Trip the circuit
    for _ in 0..3 {
        let _ = client.get("/fail", &claims).await;
    }

    // Wait for open_duration to allow half-open transition
    tokio::time::sleep(Duration::from_millis(2100)).await;

    // Probe against /fail — should fail, reopening the circuit
    let resp = client.get("/fail", &claims).await.unwrap();
    // After the probe fails, the circuit is Open again.
    // The probe itself returns the actual server response (503).
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);

    // Next call should be circuit_open again
    let resp = client.get("/fail", &claims).await.unwrap();
    let body: Value = resp.json().await.unwrap();
    assert_eq!(
        body["error"], "circuit_open",
        "circuit must be open again after failed probe"
    );

    let status = client.circuit_status("test-service");
    assert_eq!(
        status.state, "open",
        "circuit must be open after failed half-open probe"
    );

    println!("PASS: failed half-open probe correctly reopened the circuit");
}

/// Bulkhead caps concurrent outbound requests.  The 6th concurrent request
/// waits (up to bulkhead_wait=200ms) then gets a 503.
#[tokio::test]
async fn test_bulkhead_caps_concurrent_requests() {
    let (base_url, _fail_hits, _ok_hits) = start_test_server().await;

    // bulkhead_capacity=5, bulkhead_wait=200ms
    let client = test_client(base_url);
    let claims = fake_claims();

    // Saturate the bulkhead with 5 concurrent slow requests (500ms each)
    let mut handles = Vec::new();
    for _ in 0..5 {
        let c = client.clone();
        let cl = claims.clone();
        handles.push(tokio::spawn(async move { c.get("/slow", &cl).await }));
    }

    // Give the 5 requests a moment to acquire their permits
    tokio::time::sleep(Duration::from_millis(50)).await;

    // 6th request should time out waiting for a bulkhead slot
    let start = Instant::now();
    let resp = client.get("/slow", &claims).await.expect("must return Ok");
    let elapsed = start.elapsed();

    assert_eq!(
        resp.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "6th concurrent request must be rejected by bulkhead"
    );
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["error"], "service_unavailable");

    // Should have waited ~bulkhead_wait (200ms) before being rejected
    assert!(
        elapsed >= Duration::from_millis(150),
        "bulkhead rejection must wait at least ~200ms, took {:?}",
        elapsed
    );
    assert!(
        elapsed < Duration::from_millis(500),
        "bulkhead rejection must not wait too long, took {:?}",
        elapsed
    );

    // Clean up the spawned tasks
    for h in handles {
        let _ = h.await;
    }

    println!(
        "PASS: bulkhead rejected 6th concurrent request after {:?}",
        elapsed
    );
}

/// `circuit_status()` correctly reports state and serializes into the
/// ReadyResponse `circuit_breakers` field.
#[tokio::test]
async fn test_circuit_status_in_ready_response() {
    let (base_url, _fail, _ok) = start_test_server().await;
    let client = test_client(base_url);
    let claims = fake_claims();

    // Closed state
    let info = client.circuit_status("bom");
    assert_eq!(info.state, "closed");
    assert_eq!(info.consecutive_failures, 0);
    assert!(info.open_since.is_none());
    assert_eq!(info.service, "bom");

    // Trip the circuit
    for _ in 0..3 {
        let _ = client.get("/fail", &claims).await;
    }

    // Open state
    let info = client.circuit_status("bom");
    assert_eq!(info.state, "open");
    assert!(
        info.open_since.is_some(),
        "open_since must be set when open"
    );
    assert!(info.consecutive_failures > 0);

    // Verify it serializes into a ReadyResponse
    let mut ready = health::build_ready_response("production", "1.0.0", vec![]);
    ready.circuit_breakers = Some(vec![info]);

    let json = serde_json::to_value(&ready).expect("serialization must succeed");
    let cbs = json["circuit_breakers"]
        .as_array()
        .expect("circuit_breakers must be array");
    assert_eq!(cbs.len(), 1);
    assert_eq!(cbs[0]["service"], "bom");
    assert_eq!(cbs[0]["state"], "open");
    assert!(cbs[0].get("open_since").is_some());

    println!("PASS: circuit_status serializes correctly into ReadyResponse.circuit_breakers");
}
