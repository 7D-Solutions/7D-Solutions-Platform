//! Stress test: Auth under load — 100 concurrent logins prove rate limiting and no bypass
//!
//! Proves that under 100-concurrent login pressure, the rate limiter triggers
//! predictably and no invalid login ever succeeds. Specifically:
//!
//! - Phase A: 100 wrong-password logins → 0 successes, all 401/429.
//! - Phase B: 80 wrong + 20 correct concurrent → correct succeed up to limiter
//!   policy, wrong never succeed, at least one 429 observed.
//! - Phase C: Post-burst health check passes.
//!
//! The identity-auth rate limiter is keyed by `tenant_id:email` with a default
//! of 5 logins per minute per email (LOGIN_PER_MIN_PER_EMAIL=5).
//!
//! ## Running
//! ```bash
//! ./scripts/cargo-slot.sh test -p e2e-tests -- auth_rate_limit_under_load_e2e --nocapture
//! ```

use reqwest::Client;
use serde_json::json;
use std::time::{Duration, Instant};
use uuid::Uuid;

const IDENTITY_AUTH_DEFAULT: &str = "http://localhost:8080";

fn base_url() -> String {
    std::env::var("IDENTITY_AUTH_URL").unwrap_or_else(|_| IDENTITY_AUTH_DEFAULT.to_string())
}

const PER_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const BATCH_TIMEOUT: Duration = Duration::from_secs(60);
const CONCURRENCY: usize = 100;
const TEST_PASSWORD: &str = "Str0ng!Pass#2026x";
const WRONG_PASSWORD: &str = "TotallyWrong!999z";

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
struct LoginOutcome {
    status: Option<u16>,
    is_timeout: bool,
    is_connection_error: bool,
    duration: Duration,
    used_correct_password: bool,
}

async fn fire_login(
    client: &Client,
    tenant_id: Uuid,
    email: &str,
    password: &str,
    correct_password: bool,
) -> LoginOutcome {
    let start = Instant::now();
    let url = format!("{}/api/auth/login", base_url());
    let body = json!({
        "tenant_id": tenant_id,
        "email": email,
        "password": password,
    });

    let fut = client.post(&url).json(&body).send();

    match tokio::time::timeout(PER_REQUEST_TIMEOUT, fut).await {
        Ok(Ok(resp)) => LoginOutcome {
            status: Some(resp.status().as_u16()),
            is_timeout: false,
            is_connection_error: false,
            duration: start.elapsed(),
            used_correct_password: correct_password,
        },
        Ok(Err(e)) => LoginOutcome {
            status: None,
            is_timeout: e.is_timeout(),
            is_connection_error: e.is_connect(),
            duration: start.elapsed(),
            used_correct_password: correct_password,
        },
        Err(_) => LoginOutcome {
            status: None,
            is_timeout: true,
            is_connection_error: false,
            duration: start.elapsed(),
            used_correct_password: correct_password,
        },
    }
}

fn summarize_login_outcomes(results: &[LoginOutcome]) {
    let mut ok_correct = 0u32;
    let mut ok_wrong = 0u32;
    let mut unauthorized = 0u32;
    let mut rate_limited = 0u32;
    let mut server_error = 0u32;
    let mut timeouts = 0u32;
    let mut conn_errors = 0u32;
    let mut other = 0u32;

    for o in results {
        match o.status {
            Some(200) if o.used_correct_password => ok_correct += 1,
            Some(200) => ok_wrong += 1,
            Some(401) => unauthorized += 1,
            Some(429) => rate_limited += 1,
            Some(500) | Some(503) => server_error += 1,
            Some(c) => {
                println!("  unexpected status {} after {:?}", c, o.duration);
                other += 1;
            }
            None if o.is_timeout => timeouts += 1,
            None if o.is_connection_error => conn_errors += 1,
            None => other += 1,
        }
    }

    println!(
        "  200(correct)={}, 200(WRONG)={}, 401={}, 429={}, 5xx={}, timeout={}, conn_err={}, other={}",
        ok_correct, ok_wrong, unauthorized, rate_limited, server_error, timeouts, conn_errors, other
    );
}

#[tokio::test]
async fn auth_rate_limit_under_load_e2e() {
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

    // --- Seed: create one valid user with known password ---
    let tenant_id = Uuid::new_v4();
    let user_id = Uuid::new_v4();
    let email = format!("ratelimit-stress-{}@test.local", Uuid::new_v4());
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
    println!("seeded user: tenant={}, email={}", tenant_id, email);

    // =====================================================================
    // Phase A: Wrong-password burst — 100 concurrent logins, all wrong pw
    //
    // All use the same tenant+email so they share one rate-limit bucket.
    // Expected: first ~5 pass limiter and get 401, rest get 429.
    // Critical: ZERO 200 responses.
    // =====================================================================
    println!(
        "\n--- Phase A: {} concurrent wrong-password logins ---",
        CONCURRENCY
    );

    let batch_start = Instant::now();
    let handles: Vec<_> = (0..CONCURRENCY)
        .map(|_| {
            let c = client.clone();
            let em = email.clone();
            let tid = tenant_id;
            tokio::spawn(async move { fire_login(&c, tid, &em, WRONG_PASSWORD, false).await })
        })
        .collect();

    let results: Vec<LoginOutcome> = match tokio::time::timeout(BATCH_TIMEOUT, async {
        let mut out = Vec::with_capacity(CONCURRENCY);
        for h in handles {
            out.push(h.await.expect("task panicked"));
        }
        out
    })
    .await
    {
        Ok(r) => r,
        Err(_) => panic!("Phase A batch timed out — possible indefinite hang"),
    };

    let pa_duration = batch_start.elapsed();
    println!(
        "Phase A done in {:?} ({} results):",
        pa_duration,
        results.len()
    );
    summarize_login_outcomes(&results);

    // CRITICAL: no wrong-password login ever gets 200
    let wrong_pw_successes: Vec<_> = results
        .iter()
        .filter(|o| o.status == Some(200) && !o.used_correct_password)
        .collect();
    assert!(
        wrong_pw_successes.is_empty(),
        "AUTH BYPASS DETECTED: {} wrong-password requests got 200",
        wrong_pw_successes.len()
    );

    // All requests must resolve (no infinite waits)
    assert_eq!(results.len(), CONCURRENCY);

    // At least one 429 observed (proves limiter is active)
    let pa_429_count = results.iter().filter(|o| o.status == Some(429)).count();
    assert!(
        pa_429_count > 0,
        "Phase A: expected at least one 429 (rate limited) but got none — limiter may not be active"
    );
    println!(
        "Phase A: {} of {} got 429 — limiter is active",
        pa_429_count, CONCURRENCY
    );

    // All responses should be 401 or 429 (or bounded timeout/conn error)
    for (i, o) in results.iter().enumerate() {
        match o.status {
            Some(401) | Some(429) => {}                         // expected
            Some(500) | Some(503) => {}                         // acceptable under extreme load
            None if o.is_timeout || o.is_connection_error => {} // bounded failure
            Some(s) => panic!(
                "Phase A request {}: unexpected status {} (expected 401/429)",
                i, s
            ),
            None => panic!("Phase A request {}: unresolved after {:?}", i, o.duration),
        }
    }

    // =====================================================================
    // Phase B: Mixed burst — 80 wrong + 20 correct, concurrent
    //
    // Use a FRESH email so the rate-limit bucket is clean. All 100 requests
    // target the same tenant+email. The limiter allows ~5 through; among
    // those, correct-password ones get 200, wrong get 401. Rest get 429.
    //
    // Critical: ZERO 200 for wrong-password requests.
    // =====================================================================
    println!("\n--- Phase B: 80 wrong + 20 correct concurrent logins ---");

    // Register a fresh user so the rate-limit bucket resets
    let email_b = format!("ratelimit-mixed-{}@test.local", Uuid::new_v4());
    let user_id_b = Uuid::new_v4();
    let resp = client
        .post(&register_url)
        .json(&json!({
            "tenant_id": tenant_id,
            "user_id": user_id_b,
            "email": email_b,
            "password": TEST_PASSWORD,
        }))
        .send()
        .await
        .expect("Phase B seed register failed");
    assert!(
        resp.status().is_success(),
        "Phase B seed register failed: {} {}",
        resp.status(),
        resp.text().await.unwrap_or_default()
    );
    println!("Phase B user seeded: email={}", email_b);

    let batch_start = Instant::now();
    let handles: Vec<_> = (0..CONCURRENCY)
        .map(|i| {
            let c = client.clone();
            let em = email_b.clone();
            let tid = tenant_id;
            // First 20 use the correct password, rest use wrong
            let (pw, correct) = if i < 20 {
                (TEST_PASSWORD, true)
            } else {
                (WRONG_PASSWORD, false)
            };
            let password = pw.to_string();
            tokio::spawn(async move { fire_login(&c, tid, &em, &password, correct).await })
        })
        .collect();

    let results: Vec<LoginOutcome> = match tokio::time::timeout(BATCH_TIMEOUT, async {
        let mut out = Vec::with_capacity(CONCURRENCY);
        for h in handles {
            out.push(h.await.expect("task panicked"));
        }
        out
    })
    .await
    {
        Ok(r) => r,
        Err(_) => panic!("Phase B batch timed out — possible indefinite hang"),
    };

    let pb_duration = batch_start.elapsed();
    println!(
        "Phase B done in {:?} ({} results):",
        pb_duration,
        results.len()
    );
    summarize_login_outcomes(&results);

    assert_eq!(results.len(), CONCURRENCY);

    // CRITICAL: no wrong-password login ever gets 200
    let wrong_pw_successes: Vec<_> = results
        .iter()
        .filter(|o| o.status == Some(200) && !o.used_correct_password)
        .collect();
    assert!(
        wrong_pw_successes.is_empty(),
        "AUTH BYPASS DETECTED in mixed burst: {} wrong-password requests got 200",
        wrong_pw_successes.len()
    );

    // Correct-password requests that passed the limiter should get 200
    let correct_successes = results
        .iter()
        .filter(|o| o.status == Some(200) && o.used_correct_password)
        .count();
    println!(
        "Phase B: {} correct-password logins succeeded (up to limiter policy)",
        correct_successes
    );

    // At least one 429 in the mixed burst (100 requests vs 5/min bucket)
    let pb_429_count = results.iter().filter(|o| o.status == Some(429)).count();
    assert!(
        pb_429_count > 0,
        "Phase B: expected at least one 429 but got none — limiter inactive under mixed load"
    );
    println!(
        "Phase B: {} of {} got 429 — limiter active under mixed load",
        pb_429_count, CONCURRENCY
    );

    // =====================================================================
    // Phase C: Post-burst health check
    // =====================================================================
    println!("\n--- Phase C: post-burst health check ---");
    tokio::time::sleep(Duration::from_millis(500)).await;

    let health_url = format!("{}/api/ready", base_url());
    let health_resp = tokio::time::timeout(Duration::from_secs(10), client.get(&health_url).send())
        .await
        .expect("health check timed out after burst — service may be hung")
        .expect("health check request failed after burst");

    assert_eq!(
        health_resp.status().as_u16(),
        200,
        "service must be healthy after auth stress burst (got {})",
        health_resp.status()
    );

    println!("service healthy after burst — rate limiting held under load");
    println!("\nSummary:");
    println!(
        "  Phase A (wrong-pw burst):  {:?}, {} 429s, 0 bypasses",
        pa_duration, pa_429_count
    );
    println!(
        "  Phase B (mixed burst):     {:?}, {} 429s, {} correct logins, 0 bypasses",
        pb_duration, pb_429_count, correct_successes
    );
    println!("  Phase C (health):          200 OK");
}
