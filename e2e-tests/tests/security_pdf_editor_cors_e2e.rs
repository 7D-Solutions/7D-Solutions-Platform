//! E2E test: pdf-editor CORS rejection (bd-10xai)
//!
//! Proves the CORS wildcard fix (bd-1ic94) via real HTTP requests against
//! the running pdf-editor service on 127.0.0.1:8102.
//!
//! Test cases:
//! 1. Disallowed origin gets NO Access-Control-Allow-Origin header.
//! 2. Preflight OPTIONS with disallowed origin is denied.
//! 3. Allowed origin (from env) gets correct CORS headers.

use std::time::Duration;

fn pdf_editor_base_url() -> String {
    std::env::var("PDF_EDITOR_URL").unwrap_or_else(|_| "http://127.0.0.1:8102".to_string())
}

fn allowed_origin() -> Option<String> {
    let raw = std::env::var("PDF_EDITOR_CORS_ORIGINS").unwrap_or_default();
    raw.split(',')
        .map(|s| s.trim().to_string())
        .find(|s| !s.is_empty())
}

async fn wait_for_pdf_editor(base: &str) {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let mut delay = Duration::from_millis(100);

    loop {
        if let Ok(resp) = client.get(format!("{}/healthz", base)).send().await {
            if resp.status().is_success() {
                return;
            }
        }
        if tokio::time::Instant::now() >= deadline {
            panic!(
                "pdf-editor not ready after 10s at {}. \
                 Ensure the service is running (port 8102).",
                base
            );
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(1));
    }
}

#[tokio::test]
async fn security_pdf_editor_cors_disallowed_origin_rejected() {
    let base = pdf_editor_base_url();
    wait_for_pdf_editor(&base).await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{}/healthz", base))
        .header("Origin", "https://evil.example")
        .send()
        .await
        .expect("request failed");

    let acao = resp
        .headers()
        .get("access-control-allow-origin")
        .map(|v| v.to_str().unwrap_or_default().to_string());

    assert!(
        acao.as_deref() != Some("https://evil.example") && acao.as_deref() != Some("*"),
        "Disallowed origin must NOT receive a matching or wildcard \
         Access-Control-Allow-Origin header, got: {:?}",
        acao
    );

    println!("PASS: disallowed origin https://evil.example correctly rejected (header={:?})", acao);
}

#[tokio::test]
async fn security_pdf_editor_cors_preflight_disallowed_origin_denied() {
    let base = pdf_editor_base_url();
    wait_for_pdf_editor(&base).await;

    let client = reqwest::Client::new();
    let resp = client
        .request(reqwest::Method::OPTIONS, format!("{}/healthz", base))
        .header("Origin", "https://evil.example")
        .header("Access-Control-Request-Method", "POST")
        .send()
        .await
        .expect("preflight request failed");

    let acao = resp
        .headers()
        .get("access-control-allow-origin")
        .map(|v| v.to_str().unwrap_or_default().to_string());

    assert!(
        acao.as_deref() != Some("https://evil.example") && acao.as_deref() != Some("*"),
        "Preflight for disallowed origin must NOT receive a matching or wildcard \
         Access-Control-Allow-Origin header, got: {:?}",
        acao
    );

    let acam = resp.headers().get("access-control-allow-methods");
    // If origin is disallowed, no allow-methods should be granted for that origin
    println!(
        "PASS: preflight for disallowed origin denied (acao={:?}, acam={:?})",
        acao, acam
    );
}

#[tokio::test]
async fn security_pdf_editor_cors_allowed_origin_accepted() {
    let base = pdf_editor_base_url();
    wait_for_pdf_editor(&base).await;

    let origin = match allowed_origin() {
        Some(o) => o,
        None => {
            println!(
                "SKIP: PDF_EDITOR_CORS_ORIGINS not set — cannot test allowed-origin path. \
                 Set PDF_EDITOR_CORS_ORIGINS to a valid origin to enable this test."
            );
            return;
        }
    };

    let client = reqwest::Client::new();

    // Simple request with allowed origin
    let resp = client
        .get(format!("{}/healthz", base))
        .header("Origin", &origin)
        .send()
        .await
        .expect("request failed");

    let acao = resp
        .headers()
        .get("access-control-allow-origin")
        .expect("allowed origin must receive Access-Control-Allow-Origin header")
        .to_str()
        .unwrap();

    assert_eq!(
        acao, origin,
        "Access-Control-Allow-Origin must echo back the allowed origin exactly"
    );

    // Preflight with allowed origin
    let resp = client
        .request(reqwest::Method::OPTIONS, format!("{}/healthz", base))
        .header("Origin", &origin)
        .header("Access-Control-Request-Method", "POST")
        .send()
        .await
        .expect("preflight request failed");

    let acao = resp
        .headers()
        .get("access-control-allow-origin")
        .expect("preflight for allowed origin must receive Access-Control-Allow-Origin")
        .to_str()
        .unwrap();

    assert_eq!(
        acao, origin,
        "Preflight Access-Control-Allow-Origin must echo back the allowed origin"
    );

    assert!(
        resp.headers().contains_key("access-control-allow-methods"),
        "Preflight for allowed origin must include Access-Control-Allow-Methods"
    );

    println!("PASS: allowed origin {} correctly accepted with proper CORS headers", origin);
}
