//! Stress test: Large payload — 10MB body proves rejection without OOM
//!
//! Proves that:
//! 1. A single 10MB payload is rejected with 4xx (not 5xx crash)
//! 2. 20 concurrent 1MB payloads all resolve cleanly within 30s
//! 3. Post-burst health check passes — no memory leak or connection exhaustion
//!
//! Tests against the Inventory service (port 8092) which has a 2 MiB default
//! body limit. The body limit layer runs before auth/routing, so no JWT needed.
//!
//! ## Running
//! ```bash
//! ./scripts/cargo-slot.sh test -p e2e-tests -- large_payload_e2e --nocapture
//! ```

use reqwest::Client;
use std::time::{Duration, Instant};

const INVENTORY_BASE: &str = "http://127.0.0.1:8092";

const SIZE_1MB: usize = 1 * 1024 * 1024;
const SIZE_10MB: usize = 10 * 1024 * 1024;
const CONCURRENT_1MB: usize = 20;

fn make_payload(size: usize) -> Vec<u8> {
    vec![b'X'; size]
}

/// POST a large payload and return (status_code, clean_rejection).
/// status_code 0 means connection was closed/reset by server (also acceptable).
async fn post_oversized(client: &Client, url: &str, payload: Vec<u8>) -> (u16, bool) {
    let resp = client
        .post(url)
        .header("Content-Type", "application/json")
        .body(payload)
        .send()
        .await;

    match resp {
        Ok(r) => {
            let status = r.status().as_u16();
            let body = r.text().await.unwrap_or_default();

            // No stack traces in response body — detect Rust backtrace patterns
            // specifically, not broad "at /" which matches URL paths in error messages
            let has_stack_trace = body.contains("panicked at")
                || body.contains("thread '")
                || body.contains("RUST_BACKTRACE")
                || body.contains("stack backtrace:")
                || body.contains("at /rustc/")
                || body.contains("at /Users/")
                || body.contains("at /home/");

            (status, !has_stack_trace)
        }
        Err(e) => {
            let msg = format!("{}", e);
            // Connection reset/closed = server rejected before buffering full body
            let acceptable = msg.contains("connection closed")
                || msg.contains("reset by peer")
                || msg.contains("broken pipe")
                || msg.contains("channel closed");
            (0, acceptable)
        }
    }
}

async fn check_health(client: &Client) -> bool {
    let url = format!("{}/healthz", INVENTORY_BASE);
    match client.get(&url).send().await {
        Ok(r) => r.status().as_u16() == 200,
        Err(_) => false,
    }
}

#[tokio::test]
async fn large_payload_e2e() {
    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("failed to create HTTP client");

    let endpoint = format!("{}/api/inventory/items", INVENTORY_BASE);

    // --- Pre-flight ---
    println!("--- Pre-flight health check ---");
    assert!(
        check_health(&client).await,
        "inventory service must be healthy before test"
    );

    // =================================================================
    // Phase 1: Single 10MB payload — must be rejected with 4xx
    // =================================================================
    println!("\n--- Phase 1: 10MB payload rejection ---");
    let payload = make_payload(SIZE_10MB);
    let (status, clean) = post_oversized(&client, &endpoint, payload).await;
    println!("  status={}, clean={}", status, clean);

    // 4xx or connection closed (0) are acceptable. NOT 5xx.
    assert!(
        status == 0 || (400..500).contains(&status),
        "10MB payload must be rejected with 4xx (or conn closed), got {}",
        status
    );
    assert!(clean, "rejection must not expose stack traces");

    // Health check after single large payload
    assert!(
        check_health(&client).await,
        "service must be healthy after 10MB rejection"
    );
    println!("  post-rejection health: OK");

    // =================================================================
    // Phase 2: 20 concurrent 1MB payloads — all must resolve cleanly
    // =================================================================
    println!("\n--- Phase 2: {} concurrent 1MB payloads ---", CONCURRENT_1MB);
    let start = Instant::now();

    let mut handles = Vec::with_capacity(CONCURRENT_1MB);
    for i in 0..CONCURRENT_1MB {
        let c = client.clone();
        let url = endpoint.clone();
        handles.push(tokio::spawn(async move {
            let payload = make_payload(SIZE_1MB);
            let (status, clean) = post_oversized(&c, &url, payload).await;
            (i, status, clean)
        }));
    }

    let mut all_clean = true;
    let mut all_acceptable = true;
    for handle in handles {
        let (i, status, clean) = handle.await.expect("task panicked");
        // 4xx or connection closed (0) are acceptable. NOT 5xx.
        let acceptable = status == 0 || (400..500).contains(&status);
        if !acceptable {
            println!("  request {}: UNEXPECTED status {}", i, status);
            all_acceptable = false;
        }
        if !clean {
            println!("  request {}: stack trace in response", i);
            all_clean = false;
        }
    }

    let elapsed = start.elapsed();
    println!(
        "  {} requests completed in {:.2}s",
        CONCURRENT_1MB,
        elapsed.as_secs_f64()
    );

    assert!(
        elapsed < Duration::from_secs(30),
        "all concurrent requests must complete within 30s, took {:.1}s",
        elapsed.as_secs_f64()
    );
    assert!(
        all_acceptable,
        "all 1MB payloads must be rejected with 4xx (or conn closed), not 5xx"
    );
    assert!(
        all_clean,
        "no response should contain stack traces"
    );

    // =================================================================
    // Phase 3: Post-burst health check
    // =================================================================
    println!("\n--- Phase 3: Post-burst health check ---");
    assert!(
        check_health(&client).await,
        "service must be healthy after concurrent payload burst"
    );
    println!("  post-burst health: OK");

    println!("\n--- large_payload_e2e: PASSED ---");
}
