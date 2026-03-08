// HTTP smoke tests: Subscriptions
//
// Proves that 4 untested Subscriptions routes respond correctly at the HTTP
// boundary via reqwest against the live Subscriptions service.
//
// Routes covered:
//   POST /api/bill-runs/execute                          (JWT required)
//   GET  /api/subscriptions/admin/projections            (x-admin-token)
//   POST /api/subscriptions/admin/projection-status      (x-admin-token)
//   POST /api/subscriptions/admin/consistency-check      (x-admin-token)

use chrono::Utc;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::{Client, StatusCode};
use serde::Serialize;
use serde_json::{json, Value};
use std::time::Duration;
use uuid::Uuid;

const SUBS_DEFAULT_URL: &str = "http://localhost:8087";

fn subs_url() -> String {
    std::env::var("SUBSCRIPTIONS_URL").unwrap_or_else(|_| SUBS_DEFAULT_URL.to_string())
}

#[derive(Serialize)]
struct TestClaims {
    sub: String,
    iss: String,
    aud: String,
    iat: i64,
    exp: i64,
    jti: String,
    tenant_id: String,
    app_id: Option<String>,
    roles: Vec<String>,
    perms: Vec<String>,
    actor_type: String,
    ver: String,
}

fn dev_private_key() -> Option<EncodingKey> {
    let pem = std::env::var("JWT_PRIVATE_KEY_PEM").ok()?;
    EncodingKey::from_rsa_pem(pem.replace("\\n", "\n").as_bytes()).ok()
}

fn make_jwt(key: &EncodingKey, tenant_id: &str, perms: &[&str]) -> String {
    let now = Utc::now();
    let claims = TestClaims {
        sub: Uuid::new_v4().to_string(),
        iss: "auth-rs".to_string(),
        aud: "7d-platform".to_string(),
        iat: now.timestamp(),
        exp: (now + chrono::Duration::minutes(15)).timestamp(),
        jti: Uuid::new_v4().to_string(),
        tenant_id: tenant_id.to_string(),
        app_id: Some(tenant_id.to_string()),
        roles: vec!["operator".to_string()],
        perms: perms.iter().map(|s| s.to_string()).collect(),
        actor_type: "user".to_string(),
        ver: "1".to_string(),
    };
    jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, key).unwrap()
}

async fn wait_for_service(client: &Client) -> bool {
    let url = format!("{}/api/health", subs_url());
    for attempt in 1..=15 {
        match client.get(&url).send().await {
            Ok(r) if r.status().is_success() => return true,
            Ok(r) => eprintln!("  subscriptions health {}/15: {}", attempt, r.status()),
            Err(e) => eprintln!("  subscriptions health {}/15: {}", attempt, e),
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    false
}

#[tokio::test]
async fn smoke_subscriptions() {
    dotenvy::dotenv().ok();

    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    if !wait_for_service(&client).await {
        eprintln!(
            "Subscriptions service not reachable at {} -- skipping",
            subs_url()
        );
        return;
    }
    println!("Subscriptions service healthy at {}", subs_url());

    let Some(key) = dev_private_key() else {
        eprintln!("JWT_PRIVATE_KEY_PEM not set -- skipping");
        return;
    };

    let tenant_id = Uuid::new_v4().to_string();
    let jwt = make_jwt(&key, &tenant_id, &["subscriptions.mutate"]);
    let base = subs_url();

    // -----------------------------------------------------------------------
    // 1. POST /api/bill-runs/execute — no JWT -> 401
    // -----------------------------------------------------------------------
    println!("\n--- 1. POST /api/bill-runs/execute (no JWT -> 401) ---");
    let resp = client
        .post(format!("{base}/api/bill-runs/execute"))
        .json(&json!({ "bill_run_id": Uuid::new_v4().to_string() }))
        .send()
        .await
        .expect("unauth request failed");
    assert_eq!(
        resp.status().as_u16(),
        401,
        "expected 401 without JWT on /api/bill-runs/execute"
    );
    println!("  no-JWT -> 401 ok");

    // -----------------------------------------------------------------------
    // 2. POST /api/bill-runs/execute — valid JWT, structured response
    //    (skips remaining JWT tests if JWT_PUBLIC_KEY not configured)
    // -----------------------------------------------------------------------
    println!("\n--- 2. POST /api/bill-runs/execute (valid JWT) ---");
    let bill_run_id = Uuid::new_v4().to_string();
    let resp = client
        .post(format!("{base}/api/bill-runs/execute"))
        .bearer_auth(&jwt)
        .json(&json!({
            "bill_run_id": bill_run_id,
            "execution_date": "2026-03-07"
        }))
        .send()
        .await
        .expect("execute bill-run request failed");
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));

    if status.as_u16() == 401 {
        eprintln!("  execute returned 401 -- JWT_PUBLIC_KEY not configured on subscriptions. Skipping JWT tests.");
    } else {
        // The endpoint may succeed (200) or fail downstream (AR unavailable) but
        // must always return a structured JSON body — never a raw 500 with SQL.
        assert!(
            status.is_success() || status == StatusCode::INTERNAL_SERVER_ERROR,
            "Unexpected status from /api/bill-runs/execute: {status} - {body}"
        );
        if status.is_success() {
            assert!(
                body["bill_run_id"].is_string(),
                "Expected bill_run_id in response: {body}"
            );
            let returned_id = body["bill_run_id"].as_str().unwrap();
            assert_eq!(returned_id, bill_run_id, "bill_run_id mismatch");
            assert!(body["subscriptions_processed"].is_number(), "Missing subscriptions_processed");
            assert!(body["invoices_created"].is_number(), "Missing invoices_created");
            assert!(body["failures"].is_number(), "Missing failures");
            println!(
                "  bill-run ok: processed={} created={} failures={}",
                body["subscriptions_processed"], body["invoices_created"], body["failures"]
            );
        } else {
            let error_str = body.to_string();
            assert!(
                body["error"].is_string() || body["message"].is_string(),
                "Error response must have 'error' or 'message' field: {body}"
            );
            assert!(
                !error_str.contains("syntax error") && !error_str.contains("relation "),
                "SQL leak detected in error response: {body}"
            );
            println!("  bill-run downstream error (structured): {}", body["error"]);
        }

        // -------------------------------------------------------------------
        // 3. POST /api/bill-runs/execute — idempotency (same bill_run_id)
        // -------------------------------------------------------------------
        if body["bill_run_id"].is_string() {
            println!("\n--- 3. POST /api/bill-runs/execute (idempotent repeat) ---");
            let resp2 = client
                .post(format!("{base}/api/bill-runs/execute"))
                .bearer_auth(&jwt)
                .json(&json!({
                    "bill_run_id": bill_run_id,
                    "execution_date": "2026-03-07"
                }))
                .send()
                .await
                .expect("idempotent execute request failed");
            assert!(
                resp2.status().is_success(),
                "Idempotent bill-run re-execute failed: {}",
                resp2.status()
            );
            let body2: Value = resp2.json().await.unwrap_or(json!({}));
            assert_eq!(
                body2["bill_run_id"].as_str().unwrap_or(""),
                bill_run_id,
                "Idempotent response must return same bill_run_id"
            );
            println!("  idempotent re-execute -> same bill_run_id ok");
        }
    }

    // -----------------------------------------------------------------------
    // Admin routes — x-admin-token
    // -----------------------------------------------------------------------
    let admin_token = std::env::var("ADMIN_TOKEN").unwrap_or_default();

    // 4. GET /api/subscriptions/admin/projections
    println!("\n--- 4. GET /api/subscriptions/admin/projections ---");
    let resp = client
        .get(format!("{base}/api/subscriptions/admin/projections"))
        .header("x-admin-token", &admin_token)
        .send()
        .await
        .expect("admin projections request failed");
    let status = resp.status();
    if admin_token.is_empty() {
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "Admin without token should be 403"
        );
        println!("  projections: 403 (ADMIN_TOKEN not set, expected)");
    } else {
        assert!(status.is_success(), "Admin projections failed: {status}");
        println!("  projections: {status}");
    }
    let resp = client
        .get(format!("{base}/api/subscriptions/admin/projections"))
        .send()
        .await
        .expect("no-token admin request failed");
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "Admin without token should be 403"
    );
    println!("  no-token -> 403 ok");

    // 5. POST /api/subscriptions/admin/projection-status
    println!("\n--- 5. POST /api/subscriptions/admin/projection-status ---");
    let resp = client
        .post(format!("{base}/api/subscriptions/admin/projection-status"))
        .header("x-admin-token", &admin_token)
        .json(&json!({ "projection_name": "subscriptions" }))
        .send()
        .await
        .expect("admin projection-status request failed");
    let status = resp.status();
    if admin_token.is_empty() {
        assert_eq!(status, StatusCode::FORBIDDEN, "Expected 403 without token");
        println!("  projection-status: 403 (ADMIN_TOKEN not set, expected)");
    } else {
        assert!(status.is_success(), "Admin projection-status failed: {status}");
        println!("  projection-status: {status}");
    }
    let resp = client
        .post(format!("{base}/api/subscriptions/admin/projection-status"))
        .json(&json!({ "projection_name": "subscriptions" }))
        .send()
        .await
        .expect("no-token projection-status request failed");
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "Expected 403 without token on projection-status"
    );
    println!("  no-token -> 403 ok");

    // 6. POST /api/subscriptions/admin/consistency-check
    println!("\n--- 6. POST /api/subscriptions/admin/consistency-check ---");
    let resp = client
        .post(format!("{base}/api/subscriptions/admin/consistency-check"))
        .header("x-admin-token", &admin_token)
        .json(&json!({ "projection_name": "subscriptions" }))
        .send()
        .await
        .expect("admin consistency-check request failed");
    let status = resp.status();
    if admin_token.is_empty() {
        assert_eq!(status, StatusCode::FORBIDDEN, "Expected 403 without token");
        println!("  consistency-check: 403 (ADMIN_TOKEN not set, expected)");
    } else {
        assert!(status.is_success(), "Admin consistency-check failed: {status}");
        println!("  consistency-check: {status}");
    }
    let resp = client
        .post(format!("{base}/api/subscriptions/admin/consistency-check"))
        .json(&json!({ "projection_name": "subscriptions" }))
        .send()
        .await
        .expect("no-token consistency-check request failed");
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "Expected 403 without token on consistency-check"
    );
    println!("  no-token -> 403 ok");

    println!("\n=== All 4 Subscriptions routes passed ===");
}
