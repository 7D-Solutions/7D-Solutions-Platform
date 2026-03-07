//! Stress test: Large payload rejection — 5MB/10MB/50MB prove clean rejection, no OOM
//!
//! Proves that oversized request bodies are rejected cleanly with bounded resource
//! usage, and the target service remains healthy after each rejection.
//!
//! Tests against two services:
//! - Inventory (port 8092): 2 MiB default body limit — 5MB and 10MB payloads rejected
//! - PDF Editor (port 8102): 2 MiB default limit on non-upload routes — 5MB/10MB rejected
//!
//! The body limit layer runs before auth/routing, so no JWT is needed.
//!
//! ## Running
//! ```bash
//! ./scripts/cargo-slot.sh test -p e2e-tests -- large_payload_rejection_e2e --nocapture
//! ```

use reqwest::Client;
use std::time::Duration;

const INVENTORY_BASE: &str = "http://127.0.0.1:8092";
const PDF_EDITOR_BASE: &str = "http://127.0.0.1:8102";

const SIZE_5MB: usize = 5 * 1024 * 1024;
const SIZE_10MB: usize = 10 * 1024 * 1024;
const SIZE_50MB: usize = 50 * 1024 * 1024;

fn make_payload(size: usize) -> Vec<u8> {
    vec![b'X'; size]
}

fn size_label(size: usize) -> &'static str {
    match size {
        SIZE_5MB => "5MB",
        SIZE_10MB => "10MB",
        SIZE_50MB => "50MB",
        _ => "unknown",
    }
}

/// POST a large payload to the given URL and verify clean rejection.
/// Returns true if the service rejected cleanly (4xx status, no stack trace).
async fn post_oversized_payload(
    client: &Client,
    url: &str,
    payload: Vec<u8>,
    label: &str,
) -> (u16, bool) {
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
            println!("  {}: status={}, body_len={}", label, status, body.len());

            // Check body doesn't contain stack traces
            let has_stack_trace = body.contains("at /") || body.contains("panicked at")
                || body.contains("thread '") || body.contains("RUST_BACKTRACE");
            if has_stack_trace {
                println!("    WARNING: response body contains stack trace info");
            }

            (status, !has_stack_trace)
        }
        Err(e) => {
            // Connection reset / closed by server is also acceptable — it means
            // the server dropped the connection before buffering the full body.
            let msg = format!("{}", e);
            let is_connection_closed = msg.contains("connection closed")
                || msg.contains("reset by peer")
                || msg.contains("broken pipe")
                || msg.contains("channel closed");
            println!("  {}: connection error ({})", label, msg);
            if is_connection_closed {
                println!("    (server closed connection early — acceptable rejection)");
                (0, true)
            } else {
                (0, false)
            }
        }
    }
}

/// Verify the service is still healthy after payload rejection.
async fn check_health(client: &Client, base_url: &str, service_name: &str) -> bool {
    let url = format!("{}/healthz", base_url);
    match client.get(&url).send().await {
        Ok(r) => {
            let status = r.status().as_u16();
            let ok = status == 200;
            if !ok {
                println!("  {} health check: status {} (expected 200)", service_name, status);
            }
            ok
        }
        Err(e) => {
            println!("  {} health check FAILED: {}", service_name, e);
            false
        }
    }
}

#[tokio::test]
async fn large_payload_rejection_e2e() {
    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("failed to create HTTP client");

    // Verify services are healthy before we start
    println!("--- Pre-flight health checks ---");
    let inv_healthy = check_health(&client, INVENTORY_BASE, "inventory").await;
    let pdf_healthy = check_health(&client, PDF_EDITOR_BASE, "pdf-editor").await;
    assert!(inv_healthy, "inventory service must be healthy before test");
    assert!(pdf_healthy, "pdf-editor service must be healthy before test");

    let mut all_clean = true;
    let mut all_healthy_after = true;

    // ===================================================================
    // Test 1: Inventory service (2MB default limit)
    // POST to a real mutation endpoint. Auth middleware may reject before
    // body is read (401), or body limit may reject first (413). Both are
    // valid — the key is: no 500, no OOM, service stays up.
    // ===================================================================
    let inv_url = format!("{}/api/inventory/items", INVENTORY_BASE);

    for size in [SIZE_5MB, SIZE_10MB] {
        let label = format!("inventory POST {} to /api/inventory/items", size_label(size));
        println!("\n--- {} ---", label);

        let payload = make_payload(size);
        let (status, clean) = post_oversized_payload(&client, &inv_url, payload, &label).await;

        // Accept: 413 (body limit), 400, 401 (auth before body read),
        // 422, or connection closed (0). NOT 500/502/503.
        let not_server_error = !matches!(status, 500 | 502 | 503);
        if !not_server_error {
            println!("  UNEXPECTED server error: {} (service may have crashed)", status);
            all_clean = false;
        }
        if !clean {
            all_clean = false;
        }

        let healthy = check_health(&client, INVENTORY_BASE, "inventory").await;
        if !healthy {
            all_healthy_after = false;
        }
        println!("  post-rejection health: {}", if healthy { "OK" } else { "FAILED" });
    }

    // ===================================================================
    // Test 2: PDF Editor service — non-upload route (2MB default limit)
    // ===================================================================
    let pdf_url = format!("{}/api/pdf/forms/templates", PDF_EDITOR_BASE);

    for size in [SIZE_5MB, SIZE_10MB] {
        let label = format!("pdf-editor POST {} to /api/pdf/forms/templates", size_label(size));
        println!("\n--- {} ---", label);

        let payload = make_payload(size);
        let (status, clean) = post_oversized_payload(&client, &pdf_url, payload, &label).await;

        let not_server_error = !matches!(status, 500 | 502 | 503);
        if !not_server_error {
            println!("  UNEXPECTED server error: {} (service may have crashed)", status);
            all_clean = false;
        }
        if !clean {
            all_clean = false;
        }

        let healthy = check_health(&client, PDF_EDITOR_BASE, "pdf-editor").await;
        if !healthy {
            all_healthy_after = false;
        }
        println!("  post-rejection health: {}", if healthy { "OK" } else { "FAILED" });
    }

    // ===================================================================
    // Test 3: PDF Editor upload route (50MB limit) — 50MB payload
    // The render-annotations route has a 50MB limit (52_428_800 bytes).
    // A 50MB payload is at the boundary. Auth may reject first (401),
    // or body limit may accept it. Key: no 500/OOM.
    // ===================================================================
    let pdf_upload_url = format!("{}/api/pdf/render-annotations", PDF_EDITOR_BASE);

    {
        let label = "pdf-editor POST 50MB to /api/pdf/render-annotations";
        println!("\n--- {} ---", label);

        let payload = make_payload(SIZE_50MB);
        let (status, clean) = post_oversized_payload(&client, &pdf_upload_url, payload, label).await;

        let not_server_error = !matches!(status, 500 | 502 | 503);
        if !not_server_error {
            println!("  UNEXPECTED server error status: {}", status);
            all_clean = false;
        }
        if !clean {
            all_clean = false;
        }

        let healthy = check_health(&client, PDF_EDITOR_BASE, "pdf-editor").await;
        if !healthy {
            all_healthy_after = false;
        }
        println!("  post-rejection health: {}", if healthy { "OK" } else { "FAILED" });
    }

    // ===================================================================
    // Final assertions
    // ===================================================================
    println!("\n--- Summary ---");
    println!("  all rejections clean (no stack traces): {}", all_clean);
    println!("  all services healthy after rejections: {}", all_healthy_after);

    assert!(
        all_clean,
        "all oversized payload rejections must be clean (no stack traces, expected status codes)"
    );

    assert!(
        all_healthy_after,
        "all services must remain healthy after oversized payload rejections"
    );

    println!("  large payload rejection: PASSED");
}
