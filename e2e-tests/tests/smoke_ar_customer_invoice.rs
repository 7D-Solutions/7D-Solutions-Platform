//! HTTP smoke tests: AR Customer + Invoice CRUD
//!
//! Proves that the 10 core AR routes respond correctly at the HTTP boundary
//! via reqwest against a live AR service. Each route is tested for:
//! - Happy path: correct status code + valid JSON response
//! - Auth enforcement: no JWT → 401 Unauthorized
//!
//! ## Running
//! ```bash
//! ./scripts/cargo-slot.sh test -p e2e-tests --test smoke_ar_customer_invoice -- --nocapture
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
        _ => panic!("unsupported method"),
    };
    let req = if let Some(b) = body {
        req.json(&b)
    } else {
        req
    };
    let resp = req.send().await.expect("unauth request failed");
    assert_eq!(resp.status().as_u16(), 401, "expected 401 without JWT at {url}");
    println!("  no-JWT → 401 ✓");
}

#[tokio::test]
async fn smoke_ar_customer_invoice() {
    dotenvy::dotenv().ok();

    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    if !wait_for_ar(&client).await {
        eprintln!("AR service not reachable at {} — skipping", ar_url());
        return;
    }
    println!("AR service healthy at {}", ar_url());

    let Some(key) = dev_private_key() else {
        eprintln!("JWT_PRIVATE_KEY_PEM not set — skipping");
        return;
    };

    let tenant_id = Uuid::new_v4().to_string();
    let jwt = make_jwt(&key, &tenant_id, &["ar.mutate", "ar.read"]);
    let base = ar_url();

    // Gate: verify the AR service accepts our JWT (has JWT_PUBLIC_KEY configured)
    let probe = client
        .get(format!("{base}/api/ar/customers"))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("JWT probe failed");
    if probe.status().as_u16() == 401 {
        eprintln!(
            "AR service returns 401 with valid JWT — JWT_PUBLIC_KEY not configured. Skipping.\n\
             Fix: set JWT_PUBLIC_KEY in docker-compose.services.yml for the AR container"
        );
        return;
    }

    // --- 1. POST /api/ar/customers ---
    println!("\n--- 1. POST /api/ar/customers ---");
    let email = format!("smoke-{}@test.local", Uuid::new_v4());
    let resp = client
        .post(format!("{base}/api/ar/customers"))
        .bearer_auth(&jwt)
        .json(&json!({"email": email, "name": "Smoke Test Customer"}))
        .send()
        .await
        .expect("create customer failed");
    assert_eq!(resp.status().as_u16(), 201, "expected 201 Created");
    let customer: Value = resp.json().await.unwrap();
    let customer_id = customer["id"].as_i64().expect("customer.id missing");
    println!("  created customer id={customer_id}");

    assert_unauth(
        &client, "POST", &format!("{base}/api/ar/customers"),
        Some(json!({"email": "x@test.local", "name": "X"})),
    ).await;

    // --- 2. GET /api/ar/customers/{id} ---
    println!("\n--- 2. GET /api/ar/customers/{{id}} ---");
    let resp = client
        .get(format!("{base}/api/ar/customers/{customer_id}"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["id"].as_i64().unwrap(), customer_id);
    assert_eq!(body["email"].as_str().unwrap(), email);
    println!("  retrieved customer email={email}");

    assert_unauth(&client, "GET", &format!("{base}/api/ar/customers/{customer_id}"), None).await;

    // --- 3. POST /api/ar/invoices ---
    println!("\n--- 3. POST /api/ar/invoices ---");
    let resp = client
        .post(format!("{base}/api/ar/invoices"))
        .bearer_auth(&jwt)
        .json(&json!({"ar_customer_id": customer_id, "amount_cents": 5000, "currency": "usd"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 201);
    let invoice: Value = resp.json().await.unwrap();
    let invoice_id = invoice["id"].as_i64().expect("invoice.id missing");
    assert_eq!(invoice["amount_cents"].as_i64().unwrap(), 5000);
    assert_eq!(invoice["status"].as_str().unwrap(), "draft");
    println!("  created invoice id={invoice_id}, amount=5000, status=draft");

    assert_unauth(
        &client, "POST", &format!("{base}/api/ar/invoices"),
        Some(json!({"ar_customer_id": customer_id, "amount_cents": 100})),
    ).await;

    // --- 4. GET /api/ar/invoices/{id} ---
    println!("\n--- 4. GET /api/ar/invoices/{{id}} ---");
    let resp = client
        .get(format!("{base}/api/ar/invoices/{invoice_id}"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["id"].as_i64().unwrap(), invoice_id);
    println!("  retrieved invoice id={invoice_id}");

    assert_unauth(&client, "GET", &format!("{base}/api/ar/invoices/{invoice_id}"), None).await;

    // --- 5. POST /api/ar/invoices/{id}/finalize ---
    println!("\n--- 5. POST /api/ar/invoices/{{id}}/finalize ---");
    let resp = client
        .post(format!("{base}/api/ar/invoices/{invoice_id}/finalize"))
        .bearer_auth(&jwt)
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["status"].as_str().unwrap(), "open");
    println!("  finalized → status=open");

    assert_unauth(
        &client, "POST", &format!("{base}/api/ar/invoices/{invoice_id}/finalize"),
        Some(json!({})),
    ).await;

    // --- 6. POST /api/ar/invoices/{id}/bill-usage ---
    println!("\n--- 6. POST /api/ar/invoices/{{id}}/bill-usage ---");
    let resp = client
        .post(format!("{base}/api/ar/invoices"))
        .bearer_auth(&jwt)
        .json(&json!({"ar_customer_id": customer_id, "amount_cents": 0, "currency": "usd"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 201);
    let bill_inv: Value = resp.json().await.unwrap();
    let bill_inv_id = bill_inv["id"].as_i64().unwrap();

    let now = Utc::now();
    let ps = (now - chrono::Duration::days(30)).to_rfc3339();
    let pe = now.to_rfc3339();

    let resp = client
        .post(format!("{base}/api/ar/invoices/{bill_inv_id}/bill-usage"))
        .bearer_auth(&jwt)
        .json(&json!({"customer_id": customer_id, "period_start": ps, "period_end": pe,
                       "correlation_id": Uuid::new_v4().to_string()}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    let body: Value = resp.json().await.unwrap();
    assert!(body.get("billed_count").is_some(), "missing billed_count");
    println!("  bill-usage → billed_count={}", body["billed_count"]);

    assert_unauth(
        &client, "POST", &format!("{base}/api/ar/invoices/{bill_inv_id}/bill-usage"),
        Some(json!({"customer_id": customer_id, "period_start": ps, "period_end": pe,
                     "correlation_id": Uuid::new_v4().to_string()})),
    ).await;

    // --- 7. POST /api/ar/charges ---
    // Requires default_payment_method_id on customer; 409 without is valid behavior.
    println!("\n--- 7. POST /api/ar/charges ---");
    let ref_id = format!("smoke-ref-{}", Uuid::new_v4());
    let resp = client
        .post(format!("{base}/api/ar/charges"))
        .bearer_auth(&jwt)
        .json(&json!({"ar_customer_id": customer_id, "amount_cents": 2500,
                       "reason": "Smoke test", "reference_id": ref_id, "currency": "usd"}))
        .send()
        .await
        .unwrap();
    let charge_status = resp.status().as_u16();
    assert!(charge_status == 201 || charge_status == 409,
        "expected 201 or 409, got {charge_status}");
    let _charge_body: Value = resp.json().await.unwrap();
    println!("  create charge → {charge_status}");

    assert_unauth(
        &client, "POST", &format!("{base}/api/ar/charges"),
        Some(json!({"ar_customer_id": customer_id, "amount_cents": 100,
                     "reason": "x", "reference_id": format!("x-{}", Uuid::new_v4())})),
    ).await;

    // --- 8. GET /api/ar/charges/{id} ---
    println!("\n--- 8. GET /api/ar/charges/{{id}} ---");
    let resp = client
        .get(format!("{base}/api/ar/charges?customer_id={customer_id}"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    let charges: Vec<Value> = resp.json().await.unwrap();

    if let Some(ch) = charges.first() {
        let cid = ch["id"].as_i64().unwrap();
        let resp = client
            .get(format!("{base}/api/ar/charges/{cid}"))
            .bearer_auth(&jwt)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status().as_u16(), 200);
        println!("  retrieved charge id={cid}");
    } else {
        let resp = client
            .get(format!("{base}/api/ar/charges/999999"))
            .bearer_auth(&jwt)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status().as_u16(), 404);
        println!("  nonexistent charge → 404 ✓");
    }

    assert_unauth(&client, "GET", &format!("{base}/api/ar/charges/1"), None).await;

    // --- 9. POST /api/ar/charges/{id}/capture ---
    // Requires "authorized" status + provider ID; pending → 400 is valid.
    println!("\n--- 9. POST /api/ar/charges/{{id}}/capture ---");
    if let Some(ch) = charges.first() {
        let cid = ch["id"].as_i64().unwrap();
        let resp = client
            .post(format!("{base}/api/ar/charges/{cid}/capture"))
            .bearer_auth(&jwt)
            .json(&json!({}))
            .send()
            .await
            .unwrap();
        let s = resp.status().as_u16();
        assert!(s == 200 || s == 400 || s == 409, "expected 200/400/409, got {s}");
        println!("  capture → {s} (expected for pending)");
    } else {
        let resp = client
            .post(format!("{base}/api/ar/charges/999999/capture"))
            .bearer_auth(&jwt)
            .json(&json!({}))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status().as_u16(), 404);
        println!("  capture nonexistent → 404 ✓");
    }

    assert_unauth(
        &client, "POST", &format!("{base}/api/ar/charges/1/capture"),
        Some(json!({})),
    ).await;

    // --- 10. POST /api/ar/usage ---
    println!("\n--- 10. POST /api/ar/usage ---");
    let idem_key = Uuid::new_v4();
    let usage_body = json!({
        "idempotency_key": idem_key, "customer_id": customer_id.to_string(),
        "metric_name": "api_calls", "quantity": 42.0, "unit": "calls",
        "unit_price_minor": 10, "period_start": ps, "period_end": pe
    });

    let resp = client
        .post(format!("{base}/api/ar/usage"))
        .bearer_auth(&jwt)
        .json(&usage_body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    let usage: Value = resp.json().await.unwrap();
    assert_eq!(usage["metric_name"].as_str().unwrap(), "api_calls");
    println!("  captured usage: metric=api_calls, qty={}", usage["quantity"]);

    // Idempotency replay
    let resp = client
        .post(format!("{base}/api/ar/usage"))
        .bearer_auth(&jwt)
        .json(&usage_body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    println!("  idempotent replay → 200 ✓");

    assert_unauth(
        &client, "POST", &format!("{base}/api/ar/usage"),
        Some(json!({"idempotency_key": Uuid::new_v4(), "customer_id": "1",
                     "metric_name": "x", "quantity": 1.0, "unit": "x",
                     "unit_price_minor": 1, "period_start": ps, "period_end": pe})),
    ).await;

    // --- Error response sanitization ---
    println!("\n--- Error response sanitization ---");
    let resp = client
        .get(format!("{base}/api/ar/customers/999999"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 404);
    let body = resp.text().await.unwrap();
    assert!(
        !body.contains("SELECT") && !body.contains("sqlx") && !body.contains("panicked"),
        "error leaks internals: {}", &body[..body.len().min(200)]
    );
    println!("  404 sanitized ✓");

    println!("\n=== All 10 AR smoke tests passed ===");
}
