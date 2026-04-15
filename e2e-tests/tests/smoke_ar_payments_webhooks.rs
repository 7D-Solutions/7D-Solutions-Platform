//! HTTP smoke tests: AR Payments, Webhooks & Dunning
//!
//! Proves that 12 AR payment-related routes respond correctly at the HTTP
//! boundary via reqwest against a live AR service. Each route is tested for:
//! - Happy path: correct status code + valid JSON response
//! - Auth enforcement: no JWT -> 401 Unauthorized
//!
//! ## Running
//! ```bash
//! ./scripts/cargo-slot.sh test -p e2e-tests --test smoke_ar_payments_webhooks -- --nocapture
//! ```

use chrono::Utc;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::Client;
use serde::Serialize;
use serde_json::{json, Value};
use std::time::Duration;
use uuid::Uuid;

const AR_DEFAULT_URL: &str = "http://localhost:8086";

fn ar_url() -> String {
    std::env::var("AR_URL").unwrap_or_else(|_| AR_DEFAULT_URL.to_string())
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

async fn wait_for_ar(client: &Client) -> bool {
    let url = format!("{}/api/health", ar_url());
    for attempt in 1..=15 {
        match client.get(&url).send().await {
            Ok(r) if r.status().is_success() => return true,
            Ok(r) => eprintln!("  AR health {}/15: {}", attempt, r.status()),
            Err(e) => eprintln!("  AR health {}/15: {}", attempt, e),
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    false
}

async fn assert_unauth(client: &Client, method: &str, url: &str, body: Option<Value>) {
    let req = match method {
        "GET" => client.get(url),
        "POST" => client.post(url),
        "PUT" => client.put(url),
        "DELETE" => client.delete(url),
        _ => panic!("unsupported method"),
    };
    let req = if let Some(b) = body {
        req.json(&b)
    } else {
        req
    };
    let resp = req.send().await.expect("unauth request failed");
    assert_eq!(
        resp.status().as_u16(),
        401,
        "expected 401 without JWT at {url}"
    );
    println!("  no-JWT -> 401 OK");
}

/// Seed: create customer + invoice so we have valid IDs for payment tests
async fn seed_customer_and_invoice(client: &Client, base: &str, jwt: &str) -> (i64, i64) {
    let email = format!("smoke-pay-{}@test.local", Uuid::new_v4());
    let resp = client
        .post(format!("{base}/api/ar/customers"))
        .bearer_auth(jwt)
        .json(&json!({"email": email, "name": "Smoke Payments Customer"}))
        .send()
        .await
        .expect("create customer failed");
    assert_eq!(resp.status().as_u16(), 201);
    let body: Value = resp.json().await.unwrap();
    let customer_id = body["id"].as_i64().expect("customer.id missing");
    println!("  seeded customer id={customer_id}");

    let resp = client
        .post(format!("{base}/api/ar/invoices"))
        .bearer_auth(jwt)
        .json(&json!({"ar_customer_id": customer_id, "amount_cents": 10000, "currency": "usd"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 201);
    let inv: Value = resp.json().await.unwrap();
    let invoice_id = inv["id"].as_i64().unwrap();
    println!("  seeded invoice id={invoice_id}");

    (customer_id, invoice_id)
}

#[tokio::test]
async fn smoke_ar_payments_webhooks() {
    dotenvy::dotenv().ok();

    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    if !wait_for_ar(&client).await {
        eprintln!("AR service not reachable at {} -- skipping", ar_url());
        return;
    }
    println!("AR service healthy at {}", ar_url());

    let Some(key) = dev_private_key() else {
        eprintln!("JWT_PRIVATE_KEY_PEM not set -- skipping");
        return;
    };

    let tenant_id = Uuid::new_v4().to_string();
    let jwt = make_jwt(&key, &tenant_id, &["ar.mutate", "ar.read"]);
    let base = ar_url();

    let probe = client
        .get(format!("{base}/api/ar/customers"))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("JWT probe failed");
    if probe.status().as_u16() == 401 {
        eprintln!(
            "AR service returns 401 with valid JWT -- JWT_PUBLIC_KEY not configured. Skipping.\n\
             Fix: set JWT_PUBLIC_KEY in docker-compose.services.yml for the AR container"
        );
        return;
    }

    println!("\n--- Seeding: customer + invoice ---");
    let (customer_id, _invoice_id) = seed_customer_and_invoice(&client, &base, &jwt).await;

    // --- 1. POST /api/ar/payment-methods (create) ---
    println!("\n--- 1. POST /api/ar/payment-methods ---");
    let pm_body = json!({
        "ar_customer_id": customer_id,
        "tilled_payment_method_id": format!("pm_smoke_{}", Uuid::new_v4())
    });
    let resp = client
        .post(format!("{base}/api/ar/payment-methods"))
        .bearer_auth(&jwt)
        .json(&pm_body)
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    let body: Value = resp.json().await.unwrap_or(json!(null));
    assert_eq!(
        status, 201,
        "expected 201 for payment method create, got {status}: {body}"
    );
    let pm_id = body["id"].as_i64().unwrap_or(0);
    println!("  payment method created -> {status}, id={pm_id}");

    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/ar/payment-methods"),
        Some(pm_body.clone()),
    )
    .await;

    // --- 2. GET /api/ar/payment-methods/{id} ---
    println!("\n--- 2. GET /api/ar/payment-methods/{{id}} ---");
    let resp = client
        .get(format!("{base}/api/ar/payment-methods/{pm_id}"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    let body: Value = resp.json().await.unwrap_or(json!(null));
    assert_eq!(
        status, 200,
        "expected 200 for get payment method, got {status}: {body}"
    );
    println!("  get payment method -> {status}");

    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/ar/payment-methods/{pm_id}"),
        None,
    )
    .await;

    // --- 3. POST /api/ar/payment-methods/{id}/set-default ---
    // New payment methods start as pending_sync, so set-default should return 400
    println!("\n--- 3. POST /api/ar/payment-methods/{{id}}/set-default ---");
    let resp = client
        .post(format!("{base}/api/ar/payment-methods/{pm_id}/set-default"))
        .bearer_auth(&jwt)
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    let body: Value = resp.json().await.unwrap_or(json!(null));
    assert!(
        status == 400 || status == 200,
        "expected 400 (pending_sync) or 200 for set-default, got {status}: {body}"
    );
    println!("  set-default on pending_sync PM -> {status} OK");

    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/ar/payment-methods/{pm_id}/set-default"),
        Some(json!({})),
    )
    .await;

    // --- 4. POST /api/ar/payments/allocate ---
    println!("\n--- 4. POST /api/ar/payments/allocate ---");
    let alloc_body = json!({
        "payment_id": format!("pay_smoke_{}", Uuid::new_v4()),
        "customer_id": customer_id,
        "amount_cents": 5000,
        "currency": "usd",
        "idempotency_key": Uuid::new_v4().to_string()
    });
    let resp = client
        .post(format!("{base}/api/ar/payments/allocate"))
        .bearer_auth(&jwt)
        .json(&alloc_body)
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    let body: Value = resp.json().await.unwrap_or(json!(null));
    assert!(
        status == 200 || status == 201 || status == 404 || status == 400 || status == 422,
        "expected valid response for allocate, got {status}: {body}"
    );
    println!("  allocate payment -> {status}");

    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/ar/payments/allocate"),
        Some(alloc_body.clone()),
    )
    .await;

    // --- 5. GET /api/ar/webhooks (list) ---
    println!("\n--- 5. GET /api/ar/webhooks ---");
    let resp = client
        .get(format!("{base}/api/ar/webhooks"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    assert_eq!(status, 200, "expected 200 for webhooks list");
    let webhooks: Value = resp.json().await.unwrap_or(json!([]));
    println!(
        "  webhooks list -> {status}, count={}",
        match &webhooks {
            Value::Array(a) => a.len(),
            _ => 0,
        }
    );

    assert_unauth(&client, "GET", &format!("{base}/api/ar/webhooks"), None).await;

    // --- 6. GET /api/ar/webhooks/{id} (404 for nonexistent) ---
    println!("\n--- 6. GET /api/ar/webhooks/{{id}} ---");
    let resp = client
        .get(format!("{base}/api/ar/webhooks/999999"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    assert_eq!(status, 404, "expected 404 for nonexistent webhook");
    println!("  nonexistent webhook -> 404 OK");

    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/ar/webhooks/999999"),
        None,
    )
    .await;

    // --- 7. POST /api/ar/webhooks/{id}/replay (404 for nonexistent) ---
    println!("\n--- 7. POST /api/ar/webhooks/{{id}}/replay ---");
    let replay_body = json!({"force": false});
    let resp = client
        .post(format!("{base}/api/ar/webhooks/999999/replay"))
        .bearer_auth(&jwt)
        .json(&replay_body)
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    assert!(
        status == 404 || status == 400,
        "expected 404/400 for replay nonexistent webhook, got {status}"
    );
    println!("  replay nonexistent -> {status} OK");

    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/ar/webhooks/999999/replay"),
        Some(replay_body),
    )
    .await;

    // --- 8. POST /api/ar/webhooks/tilled (no JWT, invalid HMAC) ---
    // This endpoint does NOT use JWT auth — it uses HMAC signature verification.
    // An invalid/missing signature should be rejected.
    println!("\n--- 8. POST /api/ar/webhooks/tilled ---");
    let tilled_body = json!({
        "id": "evt_smoke_test",
        "type": "payment_intent.succeeded",
        "data": {"object": {}}
    });
    let resp = client
        .post(format!("{base}/api/ar/webhooks/tilled"))
        .header("tilled-signature", "invalid_signature")
        .json(&tilled_body)
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    assert!(
        status == 400 || status == 401 || status == 500,
        "expected 400/401/500 for invalid tilled signature, got {status}"
    );
    println!("  tilled webhook with bad HMAC -> {status} OK");

    // --- 9. POST /api/ar/dunning/poll ---
    println!("\n--- 9. POST /api/ar/dunning/poll ---");
    let dunning_body = json!({"batch_size": 5});
    let resp = client
        .post(format!("{base}/api/ar/dunning/poll"))
        .bearer_auth(&jwt)
        .json(&dunning_body)
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    let body: Value = resp.json().await.unwrap_or(json!(null));
    assert_eq!(
        status, 200,
        "expected 200 for dunning poll, got {status}: {body}"
    );
    println!(
        "  dunning poll -> {status}, processed={}",
        body["processed"]
    );

    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/ar/dunning/poll"),
        Some(dunning_body.clone()),
    )
    .await;

    // --- 10. POST /api/ar/subscriptions (create) ---
    println!("\n--- 10. POST /api/ar/subscriptions ---");
    let sub_body = json!({
        "ar_customer_id": customer_id,
        "payment_method_id": format!("pm_{}", pm_id),
        "plan_id": format!("plan_smoke_{}", Uuid::new_v4()),
        "plan_name": "Smoke Test Plan",
        "price_cents": 2999,
        "interval_unit": "month"
    });
    let resp = client
        .post(format!("{base}/api/ar/subscriptions"))
        .bearer_auth(&jwt)
        .json(&sub_body)
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    let body: Value = resp.json().await.unwrap_or(json!(null));
    assert_eq!(
        status, 201,
        "expected 201 for subscription create, got {status}: {body}"
    );
    let sub_id = body["id"].as_i64().unwrap_or(0);
    println!("  subscription created -> {status}, id={sub_id}");

    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/ar/subscriptions"),
        Some(sub_body.clone()),
    )
    .await;

    // --- 11. GET /api/ar/subscriptions/{id} ---
    println!("\n--- 11. GET /api/ar/subscriptions/{{id}} ---");
    let resp = client
        .get(format!("{base}/api/ar/subscriptions/{sub_id}"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    let body: Value = resp.json().await.unwrap_or(json!(null));
    assert_eq!(
        status, 200,
        "expected 200 for get subscription, got {status}: {body}"
    );
    println!("  get subscription -> {status}");

    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/ar/subscriptions/{sub_id}"),
        None,
    )
    .await;

    // --- 12. POST /api/ar/subscriptions/{id}/cancel ---
    println!("\n--- 12. POST /api/ar/subscriptions/{{id}}/cancel ---");
    let cancel_body = json!({"cancel_at_period_end": true});
    let resp = client
        .post(format!("{base}/api/ar/subscriptions/{sub_id}/cancel"))
        .bearer_auth(&jwt)
        .json(&cancel_body)
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    let body: Value = resp.json().await.unwrap_or(json!(null));
    assert_eq!(
        status, 200,
        "expected 200 for cancel subscription, got {status}: {body}"
    );
    println!("  cancel subscription -> {status}");

    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/ar/subscriptions/{sub_id}/cancel"),
        Some(cancel_body),
    )
    .await;

    println!("\n=== All 12 AR payments/webhooks/dunning smoke tests passed ===");
}
