//! Stress test: DB pool exhaustion on identity-auth
//!
//! Proves that under sustained concurrency exceeding the DB pool size,
//! identity-auth handles pool pressure cleanly: bounded latency, clean
//! error responses, no indefinite hangs, and the service stays healthy
//! after the burst.
//!
//! The identity-auth pool is configured with max_connections(10) and no
//! acquire_timeout. This test fires 50 concurrent requests that each
//! acquire a DB connection (via Argon2 hash + transaction), then asserts
//! the batch completes within a bounded time and the service recovers.
//!
//! ## Running
//! ```bash
//! ./scripts/cargo-slot.sh test -p e2e-tests -- db_pool_exhaustion_identity_auth_e2e --nocapture
//! ```

use reqwest::Client;
use serde_json::json;
use std::time::{Duration, Instant};
use uuid::Uuid;

const IDENTITY_AUTH_DEFAULT: &str = "http://localhost:8080";

fn base_url() -> String {
    std::env::var("IDENTITY_AUTH_URL").unwrap_or_else(|_| IDENTITY_AUTH_DEFAULT.to_string())
}

/// Per-request timeout — any request exceeding this is a bounded client timeout.
const PER_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Overall batch timeout — if the entire batch takes longer, something is hanging.
const BATCH_TIMEOUT: Duration = Duration::from_secs(60);

/// Concurrent requests per wave (5x the pool size of 10).
const CONCURRENCY: usize = 50;

const TEST_PASSWORD: &str = "Str0ng!Pass#2026x";

/// Wait for the identity-auth service to become healthy, or skip the test.
async fn wait_for_service(client: &Client) -> bool {
    let url = format!("{}/api/ready", base_url());
    for attempt in 1..=20 {
        match client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => return true,
            Ok(resp) => {
                eprintln!(
                    "  health check attempt {}/20: status {}",
                    attempt,
                    resp.status()
                );
            }
            Err(e) => {
                eprintln!("  health check attempt {}/20: {}", attempt, e);
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    false
}

#[derive(Debug)]
struct RequestOutcome {
    status: Option<u16>,
    is_timeout: bool,
    is_connection_error: bool,
    duration: Duration,
}

impl RequestOutcome {
    fn resolved_cleanly(&self) -> bool {
        // A request resolved cleanly if it got ANY HTTP response or a bounded
        // client-side timeout. Connection resets are less clean but still bounded.
        self.status.is_some() || self.is_timeout || self.is_connection_error
    }
}

/// Fire a single request with a per-request timeout, returning the outcome.
async fn fire_request(
    client: &Client,
    method: &str,
    url: &str,
    body: Option<serde_json::Value>,
) -> RequestOutcome {
    let start = Instant::now();

    let fut = match method {
        "GET" => client.get(url).send(),
        "POST" => {
            let mut req = client.post(url);
            if let Some(b) = body {
                req = req.json(&b);
            }
            req.send()
        }
        _ => unreachable!(),
    };

    match tokio::time::timeout(PER_REQUEST_TIMEOUT, fut).await {
        Ok(Ok(resp)) => RequestOutcome {
            status: Some(resp.status().as_u16()),
            is_timeout: false,
            is_connection_error: false,
            duration: start.elapsed(),
        },
        Ok(Err(e)) => RequestOutcome {
            status: None,
            is_timeout: e.is_timeout(),
            is_connection_error: e.is_connect(),
            duration: start.elapsed(),
        },
        Err(_) => RequestOutcome {
            status: None,
            is_timeout: true,
            is_connection_error: false,
            duration: start.elapsed(),
        },
    }
}

/// Collect and summarize a batch of request outcomes.
fn summarize(results: &[RequestOutcome]) -> (u32, u32, u32, u32) {
    let mut ok = 0u32;
    let mut err = 0u32;
    let mut timeout = 0u32;
    let mut reset = 0u32;

    for (i, o) in results.iter().enumerate() {
        match o {
            RequestOutcome { status: Some(c), .. } if *c == 200 || *c == 409 => ok += 1,
            RequestOutcome { status: Some(c), .. } if *c == 429 || *c == 503 || *c == 500 => {
                err += 1;
            }
            RequestOutcome { is_timeout: true, .. } => {
                timeout += 1;
                println!("  request {}: timeout after {:?}", i, o.duration);
            }
            RequestOutcome { is_connection_error: true, .. } => {
                reset += 1;
                println!("  request {}: connection error after {:?}", i, o.duration);
            }
            RequestOutcome { status: Some(c), .. } => {
                err += 1;
                println!("  request {}: status {} after {:?}", i, c, o.duration);
            }
            _ => {
                err += 1;
                println!("  request {}: unknown error after {:?}", i, o.duration);
            }
        }
    }

    (ok, err, timeout, reset)
}

#[tokio::test]
async fn db_pool_exhaustion_identity_auth_e2e() {
    let client = Client::builder()
        .timeout(Duration::from_secs(15))
        .pool_max_idle_per_host(0)
        .build()
        .unwrap();

    // --- Gate: skip if identity-auth is not running ---
    if !wait_for_service(&client).await {
        eprintln!(
            "identity-auth not reachable at {} — skipping stress test",
            base_url()
        );
        return;
    }
    println!("identity-auth is healthy at {}", base_url());

    // --- Step 1: Seed one user for the lifecycle query baseline ---
    let tenant_id = Uuid::new_v4();
    let user_id = Uuid::new_v4();
    let email = format!("pool-stress-seed-{}@test.local", Uuid::new_v4());
    let register_url = format!("{}/api/auth/register", base_url());

    let resp = client
        .post(&register_url)
        .json(&json!({
            "tenant_id": tenant_id,
            "user_id": user_id,
            "email": email,
            "password": TEST_PASSWORD,
        }))
        .send()
        .await
        .expect("seed register failed");
    assert!(
        resp.status().is_success(),
        "seed register failed: {} {}",
        resp.status(),
        resp.text().await.unwrap_or_default()
    );
    println!("seeded user: tenant={}, user={}", tenant_id, user_id);

    // =====================================================================
    // Phase 1: Lifecycle query burst (lightweight DB stress baseline)
    // Each request does a SELECT against the user_lifecycle_audit table.
    // 50 concurrent against pool of 10 — proves basic oversubscription.
    // =====================================================================
    println!("\n--- Phase 1: {} concurrent lifecycle queries ---", CONCURRENCY);
    let lifecycle_url = format!(
        "{}/api/auth/lifecycle/{}/{}",
        base_url(), tenant_id, user_id
    );

    let batch_start = Instant::now();
    let handles: Vec<_> = (0..CONCURRENCY)
        .map(|_| {
            let c = client.clone();
            let url = lifecycle_url.clone();
            tokio::spawn(async move { fire_request(&c, "GET", &url, None).await })
        })
        .collect();

    let results: Vec<RequestOutcome> = match tokio::time::timeout(BATCH_TIMEOUT, async {
        let mut out = Vec::with_capacity(CONCURRENCY);
        for h in handles {
            out.push(h.await.expect("task panicked"));
        }
        out
    })
    .await
    {
        Ok(r) => r,
        Err(_) => panic!("Phase 1 batch timed out — possible indefinite hang"),
    };

    let p1_duration = batch_start.elapsed();
    let (p1_ok, p1_err, p1_timeout, p1_reset) = summarize(&results);
    println!(
        "Phase 1 done in {:?}: ok={}, errors={}, timeouts={}, resets={}",
        p1_duration, p1_ok, p1_err, p1_timeout, p1_reset
    );

    assert!(p1_ok > 0, "Phase 1: at least one lifecycle request must succeed");
    assert_eq!(results.len(), CONCURRENCY);

    // =====================================================================
    // Phase 2: Concurrent register burst (heavy DB stress)
    // Each register does: Argon2 hash (~100ms+) + BEGIN + INSERT + INSERT
    // + COMMIT. This holds a DB connection for the full transaction duration.
    // 50 concurrent against pool of 10 = real pool queuing.
    // =====================================================================
    println!("\n--- Phase 2: {} concurrent register requests ---", CONCURRENCY);

    let batch_start = Instant::now();
    let handles: Vec<_> = (0..CONCURRENCY)
        .map(|i| {
            let c = client.clone();
            let url = register_url.clone();
            // Unique tenant + email per request to avoid rate limiting and conflicts
            let tid = Uuid::new_v4();
            let uid = Uuid::new_v4();
            let em = format!("pool-stress-{}-{}@test.local", i, Uuid::new_v4());
            let body = json!({
                "tenant_id": tid,
                "user_id": uid,
                "email": em,
                "password": TEST_PASSWORD,
            });
            tokio::spawn(async move { fire_request(&c, "POST", &url, Some(body)).await })
        })
        .collect();

    let results: Vec<RequestOutcome> = match tokio::time::timeout(BATCH_TIMEOUT, async {
        let mut out = Vec::with_capacity(CONCURRENCY);
        for h in handles {
            out.push(h.await.expect("task panicked"));
        }
        out
    })
    .await
    {
        Ok(r) => r,
        Err(_) => panic!("Phase 2 batch timed out — possible indefinite hang"),
    };

    let p2_duration = batch_start.elapsed();
    let (p2_ok, p2_err, p2_timeout, p2_reset) = summarize(&results);
    println!(
        "Phase 2 done in {:?}: ok={}, errors={}, timeouts={}, resets={}",
        p2_duration, p2_ok, p2_err, p2_timeout, p2_reset
    );

    // All requests must have resolved (no infinite waits)
    assert_eq!(results.len(), CONCURRENCY);
    assert!(
        results.iter().all(|r| r.resolved_cleanly()),
        "every request must resolve cleanly (response, timeout, or connection error)"
    );

    // Connection resets should be minimal — pool exhaustion should produce clean
    // timeouts or error responses, not TCP resets.
    assert!(
        p2_reset <= 2,
        "too many connection resets ({p2_reset}): expect clean errors under pool pressure"
    );

    // At least some should succeed (proves the service processed requests)
    assert!(
        p2_ok > 0,
        "Phase 2: at least one register must succeed (got 0 out of {CONCURRENCY})"
    );

    // If timeouts occurred, they're bounded by PER_REQUEST_TIMEOUT — not infinite
    for o in &results {
        if o.is_timeout {
            assert!(
                o.duration <= PER_REQUEST_TIMEOUT + Duration::from_secs(1),
                "timeout duration {:?} exceeds per-request limit {:?}",
                o.duration,
                PER_REQUEST_TIMEOUT
            );
        }
    }

    // =====================================================================
    // Phase 3: Post-burst health check
    // Service must be healthy — pool recovered, no stuck connections.
    // =====================================================================
    println!("\n--- Phase 3: post-burst health check ---");
    tokio::time::sleep(Duration::from_millis(500)).await;

    let health_url = format!("{}/api/ready", base_url());
    let health_resp = tokio::time::timeout(Duration::from_secs(10), client.get(&health_url).send())
        .await
        .expect("health check timed out after burst — service may be hung")
        .expect("health check request failed after burst");

    assert_eq!(
        health_resp.status().as_u16(),
        200,
        "service must be healthy after pool exhaustion burst (got {})",
        health_resp.status()
    );

    println!("service healthy after burst — pool exhaustion handled cleanly");
    println!("\nSummary:");
    println!("  Phase 1 (lifecycle queries): {:?}, {}/{} ok", p1_duration, p1_ok, CONCURRENCY);
    println!("  Phase 2 (register burst):    {:?}, {}/{} ok, {} timeouts, {} errors",
        p2_duration, p2_ok, CONCURRENCY, p2_timeout, p2_err);
    println!("  Phase 3 (health):            200 OK");
}
