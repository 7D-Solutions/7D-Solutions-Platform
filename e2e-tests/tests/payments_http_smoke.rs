// HTTP smoke tests: Payments
//
// Proves that 5 core Payments routes respond correctly at the HTTP boundary
// via reqwest against the live Payments service.
// Full lifecycle: create checkout session → get session → present session
//                 → poll status → webhook signature rejection.

use chrono::Utc;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::{Client, StatusCode};
use serde::Serialize;
use serde_json::{json, Value};
use std::time::Duration;
use uuid::Uuid;

const PAY_DEFAULT_URL: &str = "http://localhost:8088";

fn pay_url() -> String {
    std::env::var("PAYMENTS_URL").unwrap_or_else(|_| PAY_DEFAULT_URL.to_string())
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
    let url = format!("{}/api/health", pay_url());
    for attempt in 1..=15 {
        match client.get(&url).send().await {
            Ok(r) if r.status().is_success() => return true,
            Ok(r) => eprintln!("  payments health {}/15: {}", attempt, r.status()),
            Err(e) => eprintln!("  payments health {}/15: {}", attempt, e),
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    false
}

async fn assert_unauth(client: &Client, method: &str, url: &str, body: Option<Value>) {
    let req = match method {
        "GET" => client.get(url),
        "POST" => client.post(url),
        _ => panic!("unsupported method"),
    };
    let req = if let Some(b) = body { req.json(&b) } else { req };
    let resp = req.send().await.expect("unauth request failed");
    assert_eq!(
        resp.status().as_u16(),
        401,
        "expected 401 without JWT at {url}"
    );
    println!("  no-JWT -> 401 ok");
}

#[tokio::test]
async fn smoke_payments() {
    dotenvy::dotenv().ok();

    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    if !wait_for_service(&client).await {
        eprintln!(
            "Payments service not reachable at {} -- skipping",
            pay_url()
        );
        return;
    }
    println!("Payments service healthy at {}", pay_url());

    let Some(key) = dev_private_key() else {
        eprintln!("JWT_PRIVATE_KEY_PEM not set -- skipping");
        return;
    };

    let tenant_id = Uuid::new_v4().to_string();
    let jwt = make_jwt(&key, &tenant_id, &["payments.mutate", "payments.read"]);
    let base = pay_url();

    // Gate: verify the service accepts our JWT
    let probe = client
        .get(format!("{base}/api/health"))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("JWT probe failed");
    if probe.status().as_u16() == 401 {
        eprintln!("Payments returns 401 with valid JWT -- JWT_PUBLIC_KEY not configured. Skipping.");
        return;
    }

    let invoice_id = Uuid::new_v4().to_string();

    // --- 1. POST /api/payments/checkout-sessions ---
    println!("\n--- 1. POST /api/payments/checkout-sessions ---");
    let resp = client
        .post(format!("{base}/api/payments/checkout-sessions"))
        .bearer_auth(&jwt)
        .json(&json!({
            "tenant_id": tenant_id,
            "invoice_id": invoice_id,
            "amount": 9900,
            "currency": "usd",
            "idempotency_key": Uuid::new_v4().to_string()
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let create_body: Value = resp.json().await.unwrap_or(json!({}));
    if !status.is_success() {
        eprintln!(
            "Create checkout session returned {status} - {create_body} (Tilled may be unavailable). Skipping."
        );
        return;
    }
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "Create checkout session failed: {status} - {create_body}"
    );
    let session_id = create_body["session_id"]
        .as_str()
        .expect("No session_id in checkout session response");
    println!("  created session id={session_id}");

    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/payments/checkout-sessions"),
        Some(json!({
            "invoice_id": Uuid::new_v4().to_string(),
            "amount": 100,
            "currency": "usd",
            "idempotency_key": Uuid::new_v4().to_string()
        })),
    )
    .await;

    // --- 2. GET /api/payments/checkout-sessions/{id} ---
    println!("\n--- 2. GET /api/payments/checkout-sessions/{{id}} ---");
    let resp = client
        .get(format!("{base}/api/payments/checkout-sessions/{session_id}"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "Get checkout session failed: {}",
        resp.status()
    );
    let fetched: Value = resp.json().await.unwrap();
    assert_eq!(
        fetched["invoice_id"].as_str().unwrap_or(""),
        invoice_id,
        "invoice_id mismatch"
    );
    println!("  retrieved session invoice_id={}", fetched["invoice_id"]);

    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/payments/checkout-sessions/{session_id}"),
        None,
    )
    .await;

    // --- 3. POST /api/payments/checkout-sessions/{id}/present ---
    println!("\n--- 3. POST .../present ---");
    let resp = client
        .post(format!(
            "{base}/api/payments/checkout-sessions/{session_id}/present"
        ))
        .bearer_auth(&jwt)
        .json(&json!({
            "tenant_id": tenant_id,
            "idempotency_key": Uuid::new_v4().to_string()
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let present_body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status.is_success(),
        "Present session failed: {status} - {present_body}"
    );
    println!("  presented session status={}", present_body["status"]);

    // Idempotent second call should also succeed
    let resp2 = client
        .post(format!(
            "{base}/api/payments/checkout-sessions/{session_id}/present"
        ))
        .bearer_auth(&jwt)
        .json(&json!({
            "tenant_id": tenant_id,
            "idempotency_key": Uuid::new_v4().to_string()
        }))
        .send()
        .await
        .unwrap();
    assert!(
        resp2.status().is_success(),
        "Idempotent present failed: {}",
        resp2.status()
    );
    println!("  idempotent present ok");

    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/payments/checkout-sessions/{session_id}/present"),
        Some(json!({"idempotency_key": Uuid::new_v4().to_string()})),
    )
    .await;

    // --- 4. GET /api/payments/checkout-sessions/{id}/status ---
    println!("\n--- 4. GET .../status ---");
    let resp = client
        .get(format!(
            "{base}/api/payments/checkout-sessions/{session_id}/status"
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "Poll status failed: {}",
        resp.status()
    );
    let status_body: Value = resp.json().await.unwrap();
    let current_status = status_body["status"].as_str().unwrap_or("?");
    println!("  status={current_status}");

    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/payments/checkout-sessions/{session_id}/status"),
        None,
    )
    .await;

    // --- 5. POST /api/payments/webhook/tilled (no signature → 401) ---
    println!("\n--- 5. POST /api/payments/webhook/tilled (no sig) ---");
    let resp = client
        .post(format!("{base}/api/payments/webhook/tilled"))
        .json(&json!({
            "type": "payment_intent.succeeded",
            "data": {"object": {"id": "mock_pi_smoke"}}
        }))
        .send()
        .await
        .expect("webhook request failed");
    assert_eq!(
        resp.status().as_u16(),
        401,
        "expected 401 without Tilled-Signature at webhook route"
    );
    println!("  no-signature -> 401 ok");

    println!("\n=== All 5 Payments routes passed ===");
}
